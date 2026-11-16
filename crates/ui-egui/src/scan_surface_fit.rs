//! Select a scan patch and reconstruct a bounded CAD sheet through the mesh worker.
use crate::{SolveApp, dialogs::Dialog, mesh_lasso, selection::SelInput, ui_kit};
use serde_json::{Value, json};
use solidcraft_engine::{Sel, doc::expr::Kind as Unit};
use solidcraft_ui_text::tr;

#[derive(Clone, Debug)]
pub(crate) struct Options {
    pub selection: usize,
    feature: Option<u64>,
    refit: bool,
    kind: usize,
    tolerance: String,
    boundary_tolerance: String,
    fairing: String,
    flow: usize,
    vector: [String; 3],
}
impl Default for Options {
    fn default() -> Self {
        Self {
            selection: 0,
            feature: None,
            refit: false,
            kind: 3,
            tolerance: "0.5 mm".into(),
            boundary_tolerance: "0.5 mm".into(),
            fairing: "0.0001".into(),
            flow: 0,
            vector: ["1".into(), "0".into(), "0".into()],
        }
    }
}
const KINDS: [&str; 4] = ["plane", "cylinder", "cone", "freeform"];
fn choice(ui: &mut egui::Ui, label: &str, id: &str, selected: &mut usize, keys: &[&str]) {
    let labels: Vec<_> = keys.iter().map(|key| tr(key)).collect();
    ui_kit::choice_row(ui, tr(label), id, selected, &labels.iter().map(|s| s.as_ref()).collect::<Vec<&str>>());
}
pub(crate) fn rows(
    app: &SolveApp,
    ui: &mut egui::Ui,
    opts: &mut Options,
    lasso: &mut mesh_lasso::State,
    inputs: &[SelInput],
    angle: &mut String,
    preview: &mut Option<crate::mesh_edit_overlay::RegionPreview>,
    extra: &serde_json::Map<String, Value>,
) -> bool {
    let mut enter = false;
    if opts.refit {
        let fitted: Vec<_> = app
            .session
            .doc
            .metadata
            .0
            .keys()
            .filter_map(|key| key.strip_prefix("scan_surface_fit:")?.parse::<u64>().ok())
            .filter_map(|id| app.session.doc.feature(id).filter(|f| f.component == app.session.active_component).map(|f| (id, f.name.clone())))
            .collect();
        let mut selected = opts.feature.and_then(|id| fitted.iter().position(|(f, _)| *f == id)).map_or(0, |i| i + 1);
        let previous = selected;
        let labels: Vec<_> = std::iter::once(tr("ui.scan_fit.select_fitted")).chain(fitted.iter().map(|(_, name)| name.as_str())).collect();
        ui_kit::choice_row(ui, tr("ui.scan_fit.fitted_surface"), "scan_fit_existing", &mut selected, &labels);
        if selected != previous {
            opts.feature = None;
            if let Some((feature, _)) = selected.checked_sub(1).and_then(|i| fitted.get(i)) {
                if let Some(recipe) = app.session.doc.metadata.0.get(&format!("scan_surface_fit:{feature}")) {
                    *opts = recipe_options(*feature, &recipe["options"]);
                }
            }
        }
        if let Some(feature) = opts.feature {
            ui.label(solidcraft_ui_text::format("ui.scan_fit.refitting", &[("feature", &feature.to_string())]));
        }
    } else {
        choice(ui, "ui.scan_fit.selection", "scan_fit_selection", &mut opts.selection, &["ui.scan_fit.region", "ui.scan_fit.lasso"]);
        if opts.selection == 1 {
            mesh_lasso::controls(ui, lasso, mesh_lasso::body(inputs).unwrap_or(""), app.session.revision, "mesh.fit_surface");
        } else if let Some(region) = extra.get("auto_region").and_then(Value::as_u64) {
            ui.label(solidcraft_ui_text::format("ui.scan_fit.detected", &[("number", &(region + 1).to_string())]));
        } else {
            enter |= ui_kit::expr_row(ui, tr("ui.scan_fit.region_angle"), angle);
            ui.add(egui::Label::new(tr("ui.scan_fit.region_help")).wrap());
            crate::mesh_edit_overlay::region_controls(app, ui, inputs, angle, preview);
        }
    }
    choice(
        ui,
        "ui.scan_fit.family",
        "scan_fit_family",
        &mut opts.kind,
        &["ui.scan_fit.plane", "ui.scan_fit.cylinder", "ui.scan_fit.cone", "ui.scan_fit.freeform"],
    );
    enter |= ui_kit::expr_row(ui, tr("ui.scan_fit.target"), &mut opts.tolerance);
    if matches!(opts.kind, 0 | 3) {
        choice(
            ui,
            "ui.scan_fit.flow",
            "scan_fit_flow",
            &mut opts.flow,
            &["ui.scan_fit.auto", "ui.scan_fit.x", "ui.scan_fit.y", "ui.scan_fit.z", "ui.scan_fit.custom"],
        );
        if opts.flow == 4 {
            for (key, value) in ["ui.scan_fit.flow_x", "ui.scan_fit.flow_y", "ui.scan_fit.flow_z"].iter().zip(&mut opts.vector) {
                enter |= ui_kit::expr_row(ui, tr(key), value);
            }
        }
        ui.add(egui::Label::new(tr("ui.scan_fit.flow_help")).wrap());
    }
    if opts.kind == 3 {
        enter |= ui_kit::expr_row(ui, tr("ui.scan_fit.fairing"), &mut opts.fairing);
        ui.add(egui::Label::new(tr("ui.scan_fit.fairing_help")).wrap());
    }
    enter |= ui_kit::expr_row(ui, tr("ui.scan_fit.boundary_tolerance"), &mut opts.boundary_tolerance);
    ui.add(egui::Label::new(tr("ui.scan_fit.boundary_help")).wrap());
    enter
}
pub(crate) fn commands(app: &SolveApp, d: &Dialog, angle: &str) -> Result<Vec<(String, Value)>, String> {
    let opts = &d.scan_surface_fit;
    if opts.refit && opts.feature.is_none() {
        return Err(tr("ui.scan_fit.select_fitted").to_string());
    }
    let mut args = if let Some(feature) = opts.feature {
        json!({"feature":feature})
    } else {
        let body = mesh_lasso::body(&d.inputs).ok_or_else(|| tr("ui.scan_fit.need_scan").to_string())?;
        if opts.selection == 1 {
            d.facet_lasso.params(body, app.session.revision)?
        } else {
            let angle = app.session.doc.eval(angle, Unit::Unitless).map_err(|e| e.to_string())?;
            if !(0.0..=180.0).contains(&angle) {
                return Err(tr("ui.scan_fit.bad_angle").to_string());
            }
            if let Some(region) = d.extra.get("auto_region").and_then(Value::as_u64) {
                json!({"body":body,"revision":app.session.revision,"auto_region":region,"angle":angle})
            } else if let Some(Sel::Face { point, .. }) = d.inputs.first().and_then(|i| i.items.first()) {
                json!({"body":body,"revision":app.session.revision,"point":[point.x,point.y,point.z],"angle":angle})
            } else {
                return Err(tr("ui.scan_fit.need_region").to_string());
            }
        }
    };
    let tolerance = crate::length_input::eval(&app.session.doc, &opts.tolerance, Unit::Length).map_err(|e| e.to_string())?;
    if !tolerance.is_finite() || tolerance <= 0.0 {
        return Err(tr("ui.scan_fit.bad_target").to_string());
    }
    let fairing = if opts.kind == 3 {
        let value = app.session.doc.eval(&opts.fairing, Unit::Unitless).map_err(|e| e.to_string())?;
        if !(0.0..=1.0).contains(&value) {
            return Err(tr("ui.scan_fit.bad_fairing").to_string());
        }
        Some(value)
    } else {
        None
    };
    args["kind"] = json!(KINDS.get(opts.kind).copied().unwrap_or("freeform"));
    args["tolerance"] = json!(tolerance);
    let boundary_tolerance = crate::length_input::eval(&app.session.doc, &opts.boundary_tolerance, Unit::Length).map_err(|e| e.to_string())?;
    if !boundary_tolerance.is_finite() || boundary_tolerance < 0.0 {
        return Err(tr("ui.scan_fit.bad_boundary").to_string());
    }
    args["boundary_tolerance"] = json!(boundary_tolerance);
    if let Some(fairing) = fairing {
        args["fairing"] = json!(fairing);
    }
    let direction = if !matches!(opts.kind, 0 | 3) {
        None
    } else {
        match opts.flow {
            1 => Some([1., 0., 0.]),
            2 => Some([0., 1., 0.]),
            3 => Some([0., 0., 1.]),
            4 => Some([
                app.session.doc.eval(&opts.vector[0], Unit::Unitless).map_err(|e| e.to_string())?,
                app.session.doc.eval(&opts.vector[1], Unit::Unitless).map_err(|e| e.to_string())?,
                app.session.doc.eval(&opts.vector[2], Unit::Unitless).map_err(|e| e.to_string())?,
            ]),
            _ => None,
        }
    };
    if let Some(v) = direction {
        if v.iter().any(|x| !x.is_finite()) || v.iter().map(|x| x * x).sum::<f64>() < 1e-20 {
            return Err(tr("ui.scan_fit.bad_flow").to_string());
        }
        args["flow"] = json!(v);
    }
    if opts.feature.is_some() && (opts.flow == 0 || !matches!(opts.kind, 0 | 3)) {
        args["auto_flow"] = json!(true);
    }
    Ok(vec![(if opts.feature.is_some() { "mesh.fit_surface.refit" } else { "mesh.fit_surface" }.into(), args)])
}
impl Options {
    pub(crate) fn memory(&self, doc: &solidcraft_engine::doc::Document) -> Value {
        let tolerance = crate::length_input::eval(doc, &self.tolerance, Unit::Length).ok().filter(|v| v.is_finite() && *v > 0.).unwrap_or(0.5);
        let boundary_tolerance =
            crate::length_input::eval(doc, &self.boundary_tolerance, Unit::Length).ok().filter(|v| v.is_finite() && *v >= 0.).unwrap_or(0.5);
        let fairing = doc.eval(&self.fairing, Unit::Unitless).ok().filter(|v| (0.0..=1.0).contains(v)).unwrap_or(0.0001);
        let vector: Vec<_> = self.vector.iter().map(|v| doc.eval(v, Unit::Unitless).ok().filter(|x| x.is_finite()).unwrap_or(0.)).collect();
        json!({"selection":self.selection,"kind":self.kind,"tolerance":format!("{tolerance} mm"),"boundary_tolerance":format!("{boundary_tolerance} mm"),"fairing":fairing.to_string(),"flow":self.flow,"vector":vector})
    }
    pub(crate) fn restore_memory(&mut self, value: &Value) {
        self.selection = value["selection"].as_u64().filter(|v| *v <= 1).unwrap_or(0) as usize;
        self.kind = value["kind"].as_u64().filter(|v| *v <= 3).unwrap_or(3) as usize;
        self.flow = value["flow"].as_u64().filter(|v| *v <= 4).unwrap_or(0) as usize;
        for (target, key) in
            [(&mut self.tolerance, "tolerance"), (&mut self.boundary_tolerance, "boundary_tolerance"), (&mut self.fairing, "fairing")]
        {
            if let Some(text) = value[key].as_str().filter(|s| s.len() <= 256) {
                *target = text.into();
            }
        }
        if let Some(v) = value["vector"].as_array().filter(|a| a.len() == 3) {
            for (target, x) in self.vector.iter_mut().zip(v) {
                if let Some(x) = x.as_f64().filter(|x| x.is_finite()) {
                    *target = x.to_string();
                }
            }
        }
    }
}
pub(crate) fn report(ui: &mut egui::Ui, v: &Value) {
    if let Some(text) = v["fit_report"].as_str() {
        ui.add(egui::Label::new(text).wrap());
    }
    if let Some(within) = v["within_tolerance"].as_bool() {
        let tk = solidcraft_theme::Tokens::get();
        ui.colored_label(if within { tk.sketch_fixed } else { tk.warning }, tr(if within { "ui.scan_fit.within" } else { "ui.scan_fit.outside" }));
    }
    for (label, key) in [("ui.scan_fit.rms", "rms_mm"), ("ui.scan_fit.maximum", "max_mm")] {
        if let Some(x) = v[key].as_f64() {
            ui_kit::row(ui, tr(label), |ui| ui.label(format!("{x:.4} mm")));
        }
    }
    for (label, key) in [
        ("ui.scan_fit.controls_u", "control_points_u"),
        ("ui.scan_fit.controls_v", "control_points_v"),
        ("ui.scan_fit.loops", "boundary_loops"),
        ("ui.scan_fit.vertices", "boundary_vertices"),
    ] {
        if let Some(x) = v[key].as_u64().filter(|x| *x > 0 || !key.starts_with("control_points_")) {
            ui_kit::row(ui, tr(label), |ui| ui.label(x.to_string()));
        }
    }
    for (label, key, unit) in [
        ("ui.scan_fit.radius_start", "radius_start_mm", "mm"),
        ("ui.scan_fit.radius_end", "radius_end_mm", "mm"),
        ("ui.scan_fit.height", "height_mm", "mm"),
        ("ui.scan_fit.half_angle", "half_angle_deg", "°"),
    ] {
        if let Some(x) = v["parameters"][key].as_f64() {
            ui_kit::row(ui, tr(label), |ui| ui.label(format!("{x:.4} {unit}")));
        }
    }
    if let Some(metric) = v["metric"].as_str() {
        ui.add(egui::Label::new(metric).wrap());
    }
    ui.add(egui::Label::new(tr("ui.scan_fit.sample_note")).wrap());
}

fn recipe_options(feature: u64, recipe: &Value) -> Options {
    let mut opts = Options::default();
    opts.refit = true;
    opts.feature = Some(feature);
    opts.kind = KINDS.iter().position(|k| Some(*k) == recipe["kind"].as_str()).unwrap_or(3);
    opts.tolerance = format!("{} mm", recipe["tolerance"].as_f64().unwrap_or(0.5));
    opts.boundary_tolerance = format!("{} mm", recipe["boundary_tolerance"].as_f64().unwrap_or(0.5));
    opts.fairing = recipe["fairing"].as_f64().unwrap_or(0.0001).to_string();
    if let Some(flow) = recipe["flow"].as_array().filter(|a| a.len() == 3) {
        opts.flow = 4;
        for (field, x) in opts.vector.iter_mut().zip(flow) {
            *field = x.as_f64().unwrap_or(0.).to_string();
        }
    }
    opts
}
/// Keep saved selections in the recipe; only small user-facing options are copied into the UI.
pub(crate) fn init_refit(app: &SolveApp, opts: &mut Options) {
    opts.refit = true;
    let state = app.session.model.state();
    let feature = app.session.selection.iter().find_map(|s| {
        let id = match s {
            Sel::Feature { id } => Some(*id),
            Sel::Body { name } | Sel::Face { body: name, .. } => state.body(name).map(|b| b.feature),
            _ => None,
        }?;
        app.session.doc.metadata.0.contains_key(&format!("scan_surface_fit:{id}")).then_some(id)
    });
    if let Some(feature) = feature {
        if let Some(recipe) = app.session.doc.metadata.0.get(&format!("scan_surface_fit:{feature}")) {
            *opts = recipe_options(feature, &recipe["options"]);
        }
    }
}
/// Reopen the saved recipe, preserving its bounded selection and feature identity.
pub(crate) fn start_refit(app: &mut SolveApp, feature: u64) {
    let Some(options) = app.session.doc.metadata.0.get(&format!("scan_surface_fit:{feature}")).map(|r| recipe_options(feature, &r["options"])) else {
        app.set_status(tr("ui.scan_fit.no_recipe"), true);
        return;
    };
    app.start("mesh.fit_surface.refit");
    if let Some(d) = &mut app.dialog {
        d.scan_surface_fit = options;
        d.inputs.clear();
    }
}
