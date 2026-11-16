//! Independent planar view of an imported scan or native point cloud.
use crate::{SolveApp, ui_kit, viewport::Proj};
use serde_json::{Value, json};
use solidcore_record::Vec3;
use solidcraft_engine::{Sel, SliceGeometry};
use std::sync::{Arc, Mutex};

#[derive(Clone, PartialEq)]
enum Source {
    Mesh(String),
    Cloud(u64, String),
}
impl Source {
    fn label(&self) -> String {
        match self {
            Self::Mesh(s) => format!("Mesh · {s}"),
            Self::Cloud(_, s) => format!("Cloud · {s}"),
        }
    }
}
fn sources(app: &SolveApp) -> Vec<Source> {
    let mut sources: Vec<_> = app
        .session
        .world_state()
        .ok()
        .into_iter()
        .flat_map(|st| st.bodies.iter().filter(|b| b.body.is_mesh()).map(|b| Source::Mesh(b.name.clone())).collect::<Vec<_>>())
        .collect();
    sources.extend(solidcraft_engine::view::shown_point_clouds(&app.session).map(|c| Source::Cloud(c.id, c.name.clone())));
    sources
}
type Pending = Arc<Mutex<Option<Result<SliceGeometry, String>>>>;
pub(crate) struct State {
    source: Option<Source>,
    axis: usize,
    offset: Option<f64>,
    thickness: Option<f64>,
    context: bool,
    data: Option<SliceGeometry>,
    dirty: bool,
    pending: Option<Pending>,
    error: String,
    trace: Vec<[f64; 2]>,
    tracing: bool,
    closed: bool,
    resolution: f64,
    revision: u64,
}
impl State {
    pub(crate) fn new(app: &SolveApp) -> Self {
        let list = sources(app);
        let selected = app.session.selection.iter().find_map(|s| match s {
            Sel::Body { name } | Sel::Face { body: name, .. } => list.iter().find(|s| matches!(s,Source::Mesh(n) if n==name)).cloned(),
            _ => None,
        });
        Self {
            source: selected.or_else(|| list.first().cloned()),
            axis: 2,
            offset: None,
            thickness: None,
            context: false,
            data: None,
            dirty: true,
            pending: None,
            error: String::new(),
            trace: vec![],
            tracing: false,
            closed: false,
            resolution: 0.5,
            revision: app.session.revision,
        }
    }
    fn invalidate(&mut self, reset: bool) {
        self.dirty = true;
        self.trace.clear();
        self.error.clear();
        if reset {
            self.offset = None;
            self.thickness = None;
            self.data = None;
        }
    }
    fn work(&mut self, app: &SolveApp, ctx: &egui::Context) {
        if self.revision != app.session.revision {
            self.revision = app.session.revision;
            self.invalidate(false);
        }
        let done = self.pending.as_ref().and_then(|v| v.lock().ok()?.take());
        if let Some(result) = done {
            self.pending = None;
            if !self.dirty {
                match result {
                    Ok(g) => {
                        self.offset = Some(g.offset);
                        self.thickness = Some(g.thickness);
                        self.data = Some(g);
                    }
                    Err(e) => {
                        self.error = e;
                        self.data = None;
                    }
                }
            }
        }
        if self.pending.is_some() || !self.dirty {
            return;
        }
        let Some(source) = self.source.clone() else { return };
        let session = app.session.scratch();
        let (a, o, t) = (self.axis, self.offset, self.thickness);
        let slot: Pending = Default::default();
        self.pending = Some(slot.clone());
        self.dirty = false;
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new().name("live-slice".into()).spawn(move || {
            let (body, cloud) = match &source {
                Source::Mesh(n) => (Some(n.as_str()), None),
                Source::Cloud(id, _) => (None, Some(*id)),
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| session.scan_live_slice(body, cloud, a, o, t)))
                .map_err(|_| "Slice calculation failed".to_owned())
                .and_then(|r| r.map_err(|e| e.to_string()));
            if let Ok(mut v) = slot.lock() {
                *v = Some(result);
            }
            ctx.request_repaint();
        });
        if let Err(e) = spawned {
            self.pending = None;
            self.error = e.to_string();
        }
    }
    pub(crate) fn inspect(&self) -> Value {
        json!({"source":self.source.as_ref().map(Source::label),"axis":(["x","y","z"][self.axis]),"offset":self.offset,"thickness":self.thickness,"context":self.context,"pending":self.dirty||self.pending.is_some(),"points":self.data.as_ref().map(|g|g.count),"segments":self.data.as_ref().map(|g|g.segment_count),"trace_points":self.trace.len(),"error":self.error})
    }
}
fn button(ui: &mut egui::Ui, name: &str) -> bool {
    let r = ui_kit::button(ui, name, ui_kit::Button::Normal);
    solidcraft_ui_registry::publish_handle(&format!("slice:{name}"), r.rect.center());
    r.clicked()
}
fn drawing(ui: &mut egui::Ui, s: &mut State) {
    let Some(g) = &s.data else { return };
    let uv = [[1, 2], [0, 2], [0, 1]][s.axis];
    let (response, painter) = ui.allocate_painter(egui::vec2(ui.available_width().max(160.), 185.), egui::Sense::click());
    let rect = response.rect.shrink(5.);
    solidcraft_ui_registry::publish_handle("slice:plot", rect.center());
    let [u, v] = uv;
    let [lo, hi] = g.bounds;
    let span = (hi[u] - lo[u]).max(1e-6);
    let height = (hi[v] - lo[v]).max(1e-6);
    let scale = (f64::from(rect.width()) / span).min(f64::from(rect.height()) / height);
    let cx = (hi[u] + lo[u]) * 0.5;
    let cy = (hi[v] + lo[v]) * 0.5;
    let to_pos = |x: f64, y: f64| rect.center() + egui::vec2(((x - cx) * scale) as f32, ((cy - y) * scale) as f32);
    let pt = |p: [f64; 3]| to_pos(p[u], p[v]);
    let accent = ui.visuals().selection.stroke.color;
    painter.rect_filled(rect, 0., ui.visuals().extreme_bg_color);
    let mut dots = egui::Mesh::default();
    if s.context {
        let muted = ui.visuals().weak_text_color().gamma_multiply(0.32);
        for &p in &g.context {
            dots.add_colored_rect(egui::Rect::from_center_size(pt(p), egui::vec2(1., 1.)), muted);
        }
    }
    for &p in &g.slab {
        dots.add_colored_rect(egui::Rect::from_center_size(pt(p), egui::vec2(1.6, 1.6)), accent);
    }
    painter.add(egui::Shape::mesh(dots));
    for e in &g.segments {
        painter.line_segment([pt(e[0]), pt(e[1])], egui::Stroke::new(1.2, ui.visuals().text_color()));
    }
    if s.tracing
        && !s.dirty
        && s.pending.is_none()
        && response.clicked()
        && let Some(p) = response.interact_pointer_pos()
    {
        s.trace.push([cx + f64::from(p.x - rect.center().x) / scale, cy - f64::from(p.y - rect.center().y) / scale]);
    }
    for w in s.trace.windows(2) {
        painter.line_segment([to_pos(w[0][0], w[0][1]), to_pos(w[1][0], w[1][1])], egui::Stroke::new(2., egui::Color32::YELLOW));
    }
    for p in &s.trace {
        painter.circle_filled(to_pos(p[0], p[1]), 2.5, egui::Color32::YELLOW);
    }
}
pub(crate) fn show(app: &mut SolveApp, ctx: &egui::Context) {
    let Some(mut s) = app.live_slice.take() else { return };
    s.work(app, ctx);
    let mut action = None;
    let list = sources(app);
    let labels: Vec<String> = list.iter().map(Source::label).collect();
    let title = "Live Slice";
    let (closed, act) =
        ui_kit::Window::new(egui::Id::unique("live_slice_view"), "section", title).fields().actions("", "Close", None).show(ctx, true, |ui| {
            if list.is_empty() {
                s.source = None;
                s.data = None;
                s.trace.clear();
                s.dirty = false;
                ui.label("Insert a scan mesh or point cloud to view a slice.");
                return;
            }
            let mut source = list.iter().position(|q| Some(q) == s.source.as_ref()).unwrap_or(0);
            ui_kit::row(ui, "Source", |ui| ui_kit::dropdown(ui, "slice_source", &mut source, &labels.iter().map(String::as_str).collect::<Vec<_>>()));
            if Some(&list[source]) != s.source.as_ref() {
                s.source = Some(list[source].clone());
                s.invalidate(true);
            }
            let old = s.axis;
            ui.horizontal(|ui| {
                for (i, name) in ["X", "Y", "Z"].into_iter().enumerate() {
                    let r = ui.selectable_value(&mut s.axis, i, name);
                    solidcraft_ui_registry::publish_handle(&format!("slice:{name}"), r.rect.center());
                }
            });
            if s.axis != old {
                s.invalidate(true);
            }
            if let Some(g) = &s.data {
                let lo = g.bounds[0][s.axis];
                let hi = g.bounds[1][s.axis];
                let span = (hi - lo).max(0.1);
                let mut o = s.offset.unwrap_or(g.offset);
                let mut t = s.thickness.unwrap_or(g.thickness);
                ui_kit::row(ui, "Offset (mm)", |ui| {
                    let r = ui_kit::slider(ui, &mut o, lo..=hi.max(lo + 0.00001), Default::default());
                    solidcraft_ui_registry::publish_handle("slice:offset", r.rect.center());
                });
                ui_kit::row(ui, "Thickness (mm)", |ui| {
                    let range = (span * 0.0001).min(t)..=(span * 2.).max(t);
                    let r = ui_kit::slider(ui, &mut t, range, Default::default());
                    solidcraft_ui_registry::publish_handle("slice:thickness", r.rect.center());
                });
                if Some(o) != s.offset || Some(t) != s.thickness {
                    s.offset = Some(o);
                    s.thickness = Some(t);
                    s.invalidate(false);
                }
            }
            ui.horizontal(|ui| {
                for (val, name) in [(false, "Solo"), (true, "Context")] {
                    let r = ui.selectable_value(&mut s.context, val, name);
                    solidcraft_ui_registry::publish_handle(&format!("slice:{name}"), r.rect.center());
                }
            });
            drawing(ui, &mut s);
            if !s.error.is_empty() {
                ui.label(&s.error);
            }
            if s.dirty || s.pending.is_some() {
                ui.label("Updating slice…");
                return;
            }
            if let Some(g) = &s.data {
                ui.label(format!("{} slab points · {} section segments", g.count, g.segment_count));
            }
            match &s.source {
                Some(Source::Mesh(_)) => {
                    if button(ui, "Create Section Sketch") {
                        action = Some("section");
                    }
                }
                Some(Source::Cloud(..)) => {
                    ui_kit::row(ui, "Fit resolution (mm)", |ui| ui_kit::scalar_field(ui, &mut s.resolution, 0.00001..=10000.));
                    if button(ui, "Fit Cloud Outline") {
                        action = Some("fit");
                    }
                    let toggle = ui.checkbox(&mut s.tracing, "Trace in slice view");
                    solidcraft_ui_registry::publish_handle("slice:trace", toggle.rect.center());
                    if s.tracing {
                        let toggle = ui.checkbox(&mut s.closed, "Close trace");
                        solidcraft_ui_registry::publish_handle("slice:close_trace", toggle.rect.center());
                        if button(ui, "Undo Trace Point") {
                            s.trace.pop();
                        }
                    }
                    if button(ui, "Create Trace Sketch") {
                        action = Some("trace");
                    }
                }
                None => {}
            }
        });
    if closed || act != ui_kit::Action::None {
        return;
    }
    if let (Some(a), Some(o), Some(t), Some(source)) = (action, s.offset, s.thickness, &s.source) {
        let axis = ["x", "y", "z"][s.axis];
        let mut origin = [0.; 3];
        origin[s.axis] = o;
        let mut normal = [0.; 3];
        normal[s.axis] = 1.;
        let (id, args) = match (source, a) {
            (Source::Mesh(n), _) => ("mesh.auto_sections", json!({"body":n,"axis":axis,"offsets":[o],"tolerance":0.05})),
            (Source::Cloud(id, _), "fit") => {
                ("pointcloud.section_profile", json!({"cloud":id,"origin":origin,"normal":normal,"thickness":t,"resolution":s.resolution}))
            }
            (Source::Cloud(id, _), _) => {
                ("pointcloud.trace_slice", json!({"cloud":id,"axis":axis,"offset":o,"thickness":t,"points":s.trace,"closed":s.closed}))
            }
        };
        match app.run(id, args) {
            Ok(_) => s.trace.clear(),
            Err(e) => s.error = e,
        }
    }
    s.work(app, ctx);
    app.live_slice = Some(s);
}
pub(crate) fn paint(app: &SolveApp, painter: &egui::Painter, proj: &Proj) {
    let Some(s) = &app.live_slice else { return };
    let Some(g) = &s.data else { return };
    let color = solidcraft_theme::Tokens::get().sel_edge;
    let pos = |p: [f64; 3]| proj.to_screen(Vec3::from(p));
    if s.dirty || s.pending.is_some() {
        return;
    }
    let mut dots = egui::Mesh::default();
    for &p in g.slab.iter().step_by(g.slab.len().div_ceil(3500).max(1)) {
        if let Some(q) = pos(p) {
            dots.add_colored_rect(egui::Rect::from_center_size(q, egui::vec2(2., 2.)), color);
        }
    }
    painter.add(egui::Shape::mesh(dots));
    for e in &g.segments {
        if let (Some(a), Some(b)) = (pos(e[0]), pos(e[1])) {
            painter.line_segment([a, b], egui::Stroke::new(1.4, color));
        }
    }
}
