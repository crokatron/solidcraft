use crate::{
    SolveApp,
    dialogs::Kind,
    selection::{BoxSel, SelInput},
    viewport::Proj,
};
use serde_json::{Value, json};
use solidcore_record::Vec3;
use solidcraft_engine::{Sel, geom::prelude::*};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Default)]
struct Mask {
    ranges: Vec<[u32; 2]>,
    preview: Vec<[Vec3; 3]>,
    dots: Vec<Vec3>,
    count: usize,
}
type Pending = Arc<Mutex<Option<Result<Arc<Mask>, String>>>>;
#[derive(Clone, Debug, Default)]
pub(crate) struct State {
    body: String,
    revision: u64,
    mode: usize,
    through: bool,
    initialized: bool,
    mask: Arc<Mask>,
    pending: Option<Pending>,
    error: Option<String>,
}
impl State {
    pub(crate) fn options(&self) -> (bool, usize) {
        (if self.initialized { self.through } else { true }, self.mode)
    }
    pub(crate) fn restore_options(&mut self, through: bool, mode: usize) {
        *self = Self { through, mode: mode.min(2), initialized: true, ..Default::default() };
    }
    /// Keep the depth the user chose, but start the next cut with an empty Replace selection.
    pub(crate) fn rearm(&mut self) {
        *self = Self { through: self.through, initialized: true, ..Default::default() };
    }
    fn bind(&mut self, name: &str, revision: u64) {
        if !self.initialized || self.body != name || self.revision != revision {
            let through = self.options().0;
            *self = State { body: name.to_owned(), revision, mode: self.mode, through, initialized: true, ..Default::default() };
        }
    }
    pub(crate) fn selection(&self) -> Option<(&str, &[[u32; 2]], u64, usize)> {
        (self.mask.count > 0).then_some((&self.body, &self.mask.ranges, self.revision, Arc::as_ptr(&self.mask) as usize))
    }
    pub(crate) fn params(&self, body: &str, revision: u64) -> Result<Value, String> {
        if self.body != body || self.revision != revision || self.mask.count == 0 || self.pending.is_some() {
            return Err("Draw a lasso around the triangles to edit".into());
        }
        Ok(json!({"body":body,"revision":revision,"ranges":self.mask.ranges}))
    }
}
pub(crate) fn body(inputs: &[SelInput]) -> Option<&str> {
    match inputs.first()?.items.first()? {
        Sel::Body { name } | Sel::Face { body: name, .. } => Some(name),
        _ => None,
    }
}
pub(crate) fn enabled(app: &SolveApp) -> bool {
    app.dialog.as_ref().is_some_and(|d| match &d.kind {
        Kind::MeshEdit { command: "mesh.fit_surface", .. } => d.scan_surface_fit.selection == 1 && body(&d.inputs).is_some(),
        Kind::MeshEdit { command: "pointcloud.copy_snippet", .. } => d.scan_copy.cloud.is_some(),
        Kind::MeshEdit { command: "mesh.copy_facet_cloud" | "mesh.delete_facets" | "mesh.isolate_facets", .. } => body(&d.inputs).is_some(),
        _ => false,
    })
}
pub(crate) fn controls(ui: &mut egui::Ui, s: &mut State, name: &str, revision: u64, command: &str) {
    let point_cloud = command == "pointcloud.copy_snippet";
    s.bind(name, revision);
    let result = s.pending.as_ref().and_then(|r| r.try_lock().ok()?.take());
    if let Some(result) = result {
        s.pending = None;
        match result {
            Ok(mask) => {
                s.mask = mask;
                s.error = None
            }
            Err(e) => s.error = Some(e),
        }
    }
    ui.label(if point_cloud {
        "Draw a loop around the points to copy."
    } else if command == "mesh.fit_surface" {
        solidcraft_ui_text::tr("ui.scan_fit.lasso_help")
    } else if command == "mesh.copy_facet_cloud" {
        "Draw a loop around the mesh patch to copy."
    } else {
        "Drag a loop, then press Delete to remove the selected facets and draw again."
    });
    ui.horizontal(|ui| {
        for (i, label) in ["Replace", "Add", "Subtract"].iter().enumerate() {
            let r = ui.selectable_value(&mut s.mode, i, *label);
            solidcraft_ui_registry::publish_handle(&format!("lasso:{label}"), r.rect.center());
        }
    });
    if !point_cloud {
        let toggle = ui.checkbox(&mut s.through, "Select through scan").on_hover_text("Include front and back surfaces inside the next loop.");
        solidcraft_ui_registry::publish_handle("lasso:through", toggle.rect.center());
    }
    ui.label(if s.pending.is_some() {
        "Selecting facets…".into()
    } else {
        format!("{} {} selected", s.mask.count, if point_cloud { "points" } else { "facets" })
    });
    if let Some(error) = &s.error {
        ui.label(error);
    }
}
fn combine(old: &[[u32; 2]], new: &[[u32; 2]], mode: usize) -> Vec<[u32; 2]> {
    if mode == 0 {
        return new.to_vec();
    }
    let mut out: Vec<[u32; 2]> = Vec::new();
    if mode == 1 {
        let mut row: Vec<_> = old.iter().chain(new).copied().collect();
        row.sort_unstable();
        for [a, b] in row {
            if let Some(last) = out.last_mut()
                && a <= last[1].saturating_add(1)
            {
                last[1] = last[1].max(b)
            } else {
                out.push([a, b])
            }
        }
    } else {
        let mut j = 0;
        for &[a, b] in old {
            let mut start = a as u64;
            while j < new.len() && new[j][1] < a {
                j += 1;
            }
            let mut k = j;
            while k < new.len() && new[k][0] <= b {
                let [lo, hi] = new[k];
                if u64::from(lo) > start {
                    out.push([start as u32, lo - 1]);
                }
                start = start.max(u64::from(hi) + 1);
                k += 1;
            }
            if start <= u64::from(b) {
                out.push([start as u32, b]);
            }
        }
    }
    out
}
pub(crate) fn finish(app: &mut SolveApp, ui: &egui::Ui, proj: &Proj, shape: &BoxSel) {
    if shape.lasso.len() < 3 {
        return;
    }
    let Some(d) = app.dialog.as_mut() else { return };
    let point_cloud = matches!(d.kind, Kind::MeshEdit { command: "pointcloud.copy_snippet", .. });
    let name = if point_cloud { d.scan_copy.cloud.map(|id| id.to_string()) } else { body(&d.inputs).map(str::to_owned) };
    let Some(name) = name else { return };
    let s = &mut d.facet_lasso;
    s.bind(&name, app.session.revision);
    if s.pending.is_some() {
        return;
    }
    let old = s.mask.clone();
    let mode = s.mode;
    let through = s.through;
    let result: Pending = Default::default();
    s.pending = Some(result.clone());
    let polygon: Vec<_> = shape.lasso.iter().map(|p| [p.x as f64, p.y as f64]).collect();
    let proj = *proj;
    let session = app.session.scratch();
    let ctx = ui.ctx().clone();
    let slot = result.clone();
    let spawn = std::thread::Builder::new().name("scan-lasso".into()).spawn(move || {
        let eye = proj.cam.eye();
        let (_, _, back) = proj.cam.basis();
        let project = |p: Vec3| {
            let a = proj.to_screen(p)?;
            let distance = (eye - p).dot(back);
            // Barycentric interpolation in screen space is linear in reciprocal
            // eye depth for a perspective camera and in eye depth for an orthographic view.
            let depth = if proj.cam.fov > 0.0 {
                if distance <= 0.0 {
                    return None;
                }
                -distance.recip()
            } else {
                distance
            };
            Some([a.x as f64, a.y as f64, depth])
        };
        let answer = if point_cloud {
            session.lasso_cloud_points(&name, project, &polygon).and_then(|(new, _, shown)| {
                let ranges = combine(&old.ranges, &new, mode);
                let dots = if mode == 0 { shown } else { session.cloud_point_preview(&name, &ranges)? };
                let count = ranges.iter().map(|r| (r[1] - r[0] + 1) as usize).sum();
                Ok(Arc::new(Mask { ranges, dots, count, ..Default::default() }))
            })
        } else {
            session.lasso_scan_facets(&name, project, &polygon, !through).and_then(|(new, _, preview)| {
                let ranges = combine(&old.ranges, &new, mode);
                let preview = if mode == 0 || ranges.is_empty() {
                    if ranges.is_empty() { vec![] } else { preview }
                } else {
                    session.scan_facet_preview(&name, &ranges)?
                };
                let count = ranges.iter().map(|r| r[1] as usize - r[0] as usize + 1).sum();
                Ok(Arc::new(Mask { ranges, preview, count, ..Default::default() }))
            })
        }
        .map_err(|e| e.to_string());
        if let Ok(mut data) = slot.lock() {
            *data = Some(answer);
        }
        ctx.request_repaint();
    });
    if let Err(e) = spawn {
        s.pending = None;
        s.error = Some(e.to_string());
    }
}
pub(crate) fn paint(app: &SolveApp, painter: &egui::Painter, proj: &Proj) {
    if !enabled(app) {
        return;
    }
    let Some(d) = &app.dialog else { return };
    let s = &d.facet_lasso;
    if s.revision != app.session.revision {
        return;
    }
    if matches!(d.kind, Kind::MeshEdit { command: "pointcloud.copy_snippet", .. }) {
        if d.scan_copy.cloud.is_some_and(|id| id.to_string() == s.body) {
            for &p in &s.mask.dots {
                if let Some(xy) = proj.to_screen(p) {
                    painter.circle_filled(xy, 2.5, solidcraft_theme::Tokens::get().sel_edge);
                }
            }
        }
        return;
    }
    if Some(s.body.as_str()) != body(&d.inputs) {
        return;
    }
    if app.dense_lasso.as_ref().is_some_and(|layer| layer.ready_for(app)) {
        return;
    }
    let mut layer = egui::Mesh::default();
    let color = solidcraft_theme::Tokens::get().sel_edge;
    for face in &s.mask.preview {
        if let [Some(a), Some(b), Some(c)] = face.map(|p| proj.to_screen(p)) {
            let i = layer.vertices.len() as u32;
            for p in [a, b, c] {
                layer.colored_vertex(p, color);
            }
            layer.add_triangle(i, i + 1, i + 2);
        }
    }
    painter.add(egui::Shape::mesh(layer));
}
