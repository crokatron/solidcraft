use crate::{
    SolveApp,
    dialogs::{Dialog, Kind},
    gizmo::{self, Triad},
    viewport::Proj,
};
use egui::{Painter, Stroke};
use solidcore_record::Vec3;
use solidcraft_engine::{Sel, geom::prelude::*};
use solidcraft_theme::Tokens;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Pose {
    pub normal: Vec3,
    pub pivot: Vec3,
}

fn geometry(app: &SolveApp, d: &Dialog) -> Option<(Pose, f64, f64)> {
    let Kind::MeshEdit { command: "mesh.trim_plane" | "mesh.section", plane, ref amount, side, .. } = d.kind else { return None };
    let name = match d.inputs.first()?.items.first()? {
        Sel::Body { name } => name,
        Sel::Face { body, .. } => body,
        _ => return None,
    };
    let state = crate::open_meshing::shown_world(app);
    let bounds = crate::viewport::body_bounds(app, &state);
    let bbox = &bounds.iter().find(|(n, b)| n == name && !b.is_empty())?.1;
    let pose = d.cut_pose.unwrap_or(Pose { normal: *[Vec3::Z, Vec3::Y, Vec3::X].get(plane)?, pivot: bbox.center() });
    let offset = crate::length_input::eval(&app.session.doc, amount, solidcraft_engine::doc::expr::Kind::Length).ok()?;
    Some((
        Pose { pivot: pose.pivot + pose.normal * (offset - pose.normal.dot(pose.pivot)), ..pose },
        bbox.diagonal().max(1.),
        if side == 0 { 1. } else { -1. },
    ))
}
fn tip(proj: &Proj, p: Pose, sign: f64) -> Option<(egui::Pos2, egui::Pos2)> {
    let anchor = proj.to_screen(p.pivot)?;
    let unit = (proj.cam.half_height() * 2. / proj.rect.height().max(1.) as f64).max(1e-8);
    let diff = proj.to_screen(p.pivot + p.normal * sign * unit * 155.)? - anchor;
    let direction = if diff.length() < 12. { egui::vec2(0., -1.) } else { diff.normalized() };
    Some((anchor, anchor + direction * 152.))
}
fn triad(p: Pose) -> Triad {
    Triad { center: p.pivot, axes: [Vec3::X, Vec3::Y, Vec3::Z], translate: true, rotate: true }
}
pub(crate) fn owns_pointer(app: &SolveApp, proj: &Proj, pt: egui::Pos2) -> bool {
    app.dialog
        .as_ref()
        .and_then(|d| geometry(app, d))
        .is_some_and(|(p, _, s)| gizmo::contains_pointer(proj, &triad(p), pt) || tip(proj, p, s).is_some_and(|(_, t)| t.distance(pt) < 15.))
}
pub(crate) fn show(app: &SolveApp, d: &mut Dialog, ui: &mut egui::Ui, painter: &Painter, proj: &Proj) {
    let Some((p, size, sign)) = geometry(app, d) else { return };
    let id = egui::Id::unique("mesh_cut_pose");
    let axis = if p.normal.x.abs() < 0.8 { Vec3::X } else { Vec3::Y };
    let Some(u) = (axis - p.normal * p.normal.dot(axis)).normalized() else { return };
    let v = p.normal.cross(u);
    let half = size * 0.48;
    let corners = [-u - v, u - v, u + v, -u + v].map(|a| proj.to_screen(p.pivot + a * half));
    let color = Tokens::get().sel_edge;
    if let [Some(a), Some(b), Some(c), Some(e)] = corners {
        painter.extend(crate::viewport::projected_polygon(&[a, b, c, e], color.gamma_multiply(0.13), Stroke::new(1.5, color)));
    }
    let mut new_pose = None;
    if let Some(g) = gizmo::show(ui, painter, proj, id, &triad(p), crate::canvas::snap_step(proj.cam.half_height())) {
        if g.started {
            ui.data_mut(|m| m.insert_temp(id.with("base"), p));
        }
        let base = ui.data(|m| m.get_temp::<Pose>(id.with("base"))).unwrap_or(p);
        new_pose = Some(match g.change {
            gizmo::Change::Translate(delta) => Pose { pivot: base.pivot + delta, ..base },
            gizmo::Change::Rotate { axis, angle } => {
                Pose { normal: gizmo::apply(&gizmo::rotation([Vec3::X, Vec3::Y, Vec3::Z][axis], angle), base.normal), ..base }
            }
        });
    }
    if let Some((center, end)) = tip(proj, p, sign) {
        let direction = (end - center).normalized();
        painter.arrow(end - direction * 33., direction * 33., Stroke::new(3.0, color));
        let grab = end - direction * 4.;
        solidcraft_ui_registry::publish_handle("cut:depth", grab);
        let response = ui.interact(egui::Rect::from_center_size(grab, egui::vec2(28., 28.)), id.with("depth"), egui::Sense::drag());
        if response.drag_started() {
            ui.data_mut(|m| m.insert_temp(id.with("base"), p));
        }
        if response.dragged() {
            let base = ui.data(|m| m.get_temp::<Pose>(id.with("base"))).unwrap_or(p);
            let scale = proj.cam.half_height() * 2. / proj.rect.height().max(1.) as f64;
            let probe = scale * 16.;
            let per_mm = proj
                .to_screen(base.pivot + base.normal * sign * probe)
                .zip(proj.to_screen(base.pivot))
                .map(|(a, b)| (a - b).length() as f64 / probe)
                .unwrap_or(0.);
            let mm = if per_mm * scale > 0.15 { 1. / per_mm } else { scale };
            let delta = response.total_drag_delta().unwrap_or_default().dot(direction) as f64 * mm * sign;
            new_pose = Some(Pose { pivot: base.pivot + base.normal * delta, ..base });
            crate::tool_modifiers::set_manipulator_dragging(true);
        }
    }
    if let Some(pose) = new_pose {
        if let Kind::MeshEdit { amount, .. } = &mut d.kind {
            *amount = crate::length_input::model(&app.session.doc, pose.normal.dot(pose.pivot));
        }
        d.cut_pose = Some(pose);
    }
}
