//! Commands that derive independent clouds and editable profile sketches from scan data.
use crate::{SolveApp, dialogs::Dialog, mesh_lasso, selection::SelInput, ui_kit};
use serde_json::{Value, json};
use solidcraft_engine::doc::expr::Kind as Unit;

#[derive(Clone, Debug)]
pub(crate) struct Options {
    pub cloud: Option<u64>,
    pub name: String,
    axis: usize,
    count: String,
    offsets: String,
    tolerance: String,
    hide: bool,
    close_open: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            cloud: None,
            name: String::new(),
            axis: 0,
            count: "5".into(),
            offsets: String::new(),
            tolerance: "0.05 mm".into(),
            hide: false,
            close_open: false,
        }
    }
}
const AXES: [&str; 4] = ["Auto", "X", "Y", "Z"];
pub(crate) fn rows(app: &SolveApp, ui: &mut egui::Ui, cmd: &str, opts: &mut Options, lasso: &mut mesh_lasso::State, inputs: &[SelInput]) -> bool {
    let mut enter = false;
    match cmd {
        "pointcloud.copy_snippet" => {
            let clouds: Vec<_> = solidcraft_engine::view::shown_point_clouds(&app.session).collect();
            if clouds.is_empty() {
                ui.label("Show a source point cloud in the Browser.");
                opts.cloud = None;
                return false;
            }
            let names: Vec<_> = clouds.iter().map(|c| c.name.as_str()).collect();
            let mut i = clouds.iter().position(|c| Some(c.id) == opts.cloud).unwrap_or(0);
            ui_kit::choice_row(ui, "Source cloud", "snippet_source", &mut i, &names);
            let c = clouds[i];
            opts.cloud = Some(c.id);
            mesh_lasso::controls(ui, lasso, &c.id.to_string(), app.session.revision, cmd);
        }
        "mesh.copy_facet_cloud" => mesh_lasso::controls(ui, lasso, mesh_lasso::body(inputs).unwrap_or(""), app.session.revision, cmd),
        "mesh.auto_sections" => {
            ui_kit::choice_row(ui, "Section axis", "scan_section_axis", &mut opts.axis, &AXES);
            if opts.offsets.trim().is_empty() {
                let eval = |s: &str| app.session.doc.eval(s, Unit::Unitless).ok();
                enter |= ui_kit::count_row(ui, "Sections", &mut opts.count, &ui_kit::Count { min: 1, max: 32, eval: &eval });
            }
            enter |= ui_kit::hint_row(ui, "Station offsets", &mut opts.offsets, "Suggested, or e.g. 5 mm, 10 mm");
            enter |= ui_kit::expr_row(ui, "Profile tolerance", &mut opts.tolerance);
            let close = ui
                .checkbox(&mut opts.close_open, "Close open contours")
                .on_hover_text("Add a straight closing edge between each open contour’s ends across missing scan data.");
            solidcraft_ui_registry::publish_handle("dialog:Close open contours", close.rect.center());
            let r = ui.checkbox(&mut opts.hide, "Hide source scan for tracing");
            solidcraft_ui_registry::publish_handle("dialog:Hide source scan for tracing", r.rect.center());
        }
        _ => {}
    }
    enter |= ui_kit::hint_row(ui, "Name", &mut opts.name, "Automatic");
    enter
}
pub(crate) fn commands(app: &SolveApp, d: &Dialog, cmd: &str) -> Result<Vec<(String, Value)>, String> {
    let opts = &d.scan_copy;
    let mut args = if cmd == "pointcloud.copy_snippet" {
        let id = opts.cloud.ok_or("pick a visible source cloud")?;
        let mut mask = d.facet_lasso.params(&id.to_string(), app.session.revision)?;
        mask.as_object_mut().ok_or("invalid point selection")?.remove("body");
        mask["cloud"] = json!(id);
        mask
    } else {
        let body = mesh_lasso::body(&d.inputs).ok_or("pick the scan body")?;
        if cmd == "mesh.copy_facet_cloud" { d.facet_lasso.params(body, app.session.revision)? } else { json!({"body":body}) }
    };
    if !opts.name.trim().is_empty() {
        args["name"] = json!(opts.name.trim());
    }
    if cmd == "mesh.auto_sections" {
        args["close_open"] = json!(opts.close_open);
        args["axis"] = json!(AXES[opts.axis].to_ascii_lowercase());
        let length = |s: &str| crate::length_input::eval(&app.session.doc, s.trim(), Unit::Length).map_err(|e| e.to_string());
        if opts.offsets.trim().is_empty() {
            let count = app.session.doc.eval(&opts.count, Unit::Unitless).map_err(|e| e.to_string())?;
            if !(1.0..=32.0).contains(&count) || count.fract() != 0.0 {
                return Err("Choose 1 to 32 sections".into());
            }
            args["count"] = json!(count as usize);
        } else {
            let offsets: Vec<_> = opts.offsets.split(',').map(length).collect::<Result<_, _>>()?;
            if offsets.len() > 32 {
                return Err("Enter up to 32 station offsets".into());
            }
            args["offsets"] = json!(offsets);
        }
        let tol = length(&opts.tolerance)?;
        if !tol.is_finite() || tol < 0.0 {
            return Err("Enter a nonnegative profile tolerance".into());
        }
        args["tolerance"] = json!(tol);
    }
    Ok(vec![(cmd.to_owned(), args)])
}

/// A worker keeps visibility changes made in the live viewport while it computes.
pub(crate) fn hide_source_after_sections(app: &mut SolveApp, d: &Dialog) -> Result<(), String> {
    if matches!(d.kind, crate::dialogs::Kind::MeshEdit { command: "mesh.auto_sections", .. })
        && d.scan_copy.hide
        && let Some(body) = mesh_lasso::body(&d.inputs)
    {
        app.run("browser.visibility", json!({"items":[body],"visible":false}))?;
    }
    Ok(())
}

impl Options {
    pub(crate) fn memory(&self, doc: &solidcraft_engine::doc::Document) -> Value {
        let length = |v: &str| crate::length_input::eval(doc, v, Unit::Length).ok().map(|v| format!("{v} mm"));
        let count = doc.eval(&self.count, Unit::Unitless).ok().map(|n| n.to_string()).unwrap_or_else(|| "5".into());
        let offsets = if self.offsets.trim().is_empty() {
            String::new()
        } else {
            self.offsets.split(',').map(|v| length(v.trim())).collect::<Option<Vec<_>>>().map(|v| v.join(", ")).unwrap_or_default()
        };
        json!({"axis":self.axis,"count":count,"offsets":offsets,"tolerance":length(&self.tolerance).unwrap_or_else(||"0.05 mm".into()),"hide":self.hide,"close_open":self.close_open})
    }
    pub(crate) fn restore_memory(&mut self, value: &Value) {
        if let Some(n) = value["axis"].as_u64().filter(|n| *n <= 3) {
            self.axis = n as usize;
        }
        for (key, text) in [("count", &mut self.count), ("offsets", &mut self.offsets), ("tolerance", &mut self.tolerance)] {
            if let Some(s) = value[key].as_str().filter(|s| s.len() <= 4096) {
                *text = s.into();
            }
        }
        if let Some(b) = value["hide"].as_bool() {
            self.hide = b;
        }
        if let Some(b) = value["close_open"].as_bool() {
            self.close_open = b;
        }
    }
}
