//! Scan alignments whose references are specific positions clicked on the scanned surface.
use crate::{selection::SelInput, viewport::Proj};
use serde_json::{Value, json};
use solidcraft_engine::Sel;

pub(crate) fn point_labels(cmd: &str) -> Option<&'static [&'static str]> {
    match cmd {
        "mesh.align_landmarks" => Some(&["Moving 1", "Fixed 1", "Moving 2", "Fixed 2", "Moving 3", "Fixed 3"]),
        "mesh.symmetry_plane" => Some(&["Left 1", "Right 1", "Left 2", "Right 2", "Left 3", "Right 3"]),
        "mesh.align_datum" => Some(&["Origin", "Point on +X", "Point toward +Y"]),
        _ => None,
    }
}
/// Enforce the same scan within each side of the correspondence, also when replacing a pick.
pub(crate) fn accepts_body(cmd: &str, inputs: &[SelInput], at: usize, name: &str) -> bool {
    if point_labels(cmd).is_none() {
        return true;
    }
    inputs.iter().enumerate().all(|(i, input)| {
        if i == at {
            return true;
        }
        let Some(Sel::Face { body, .. }) = input.items.first() else {
            return true;
        };
        let same_side = matches!(cmd, "mesh.align_datum" | "mesh.symmetry_plane") || i % 2 == at % 2;
        (name == body) == same_side
    })
}

pub(crate) fn point_params(cmd: &str, inputs: &[SelInput]) -> Result<Value, String> {
    let mut points = Vec::new();
    let mut bodies = Vec::new();
    for (i, input) in inputs.iter().enumerate() {
        let Some(Sel::Face { body, point, .. }) = input.items.first() else {
            return Err(format!("Pick {}", input.label));
        };
        if !accepts_body(cmd, inputs, i, body) {
            return Err(format!("Pick {} on the matching scan", input.label));
        }
        points.push([point.x, point.y, point.z]);
        bodies.push(body);
    }
    match (cmd, points.as_slice()) {
        ("mesh.align_landmarks", [m1, f1, m2, f2, m3, f3]) => {
            Ok(json!({"moving":bodies[0],"fixed":bodies[1],"moving_points":[m1,m2,m3],"fixed_points":[f1,f2,f3]}))
        }
        ("mesh.align_datum", [origin, x, xy]) => Ok(json!({"moving":bodies[0],"origin":origin,"x_point":x,"xy_point":xy})),
        _ => Err("Complete the scan alignment references".into()),
    }
}

pub(crate) fn paint(cmd: &str, inputs: &[SelInput], painter: &egui::Painter, proj: &Proj) {
    if point_labels(cmd).is_none() {
        return;
    }
    for (i, input) in inputs.iter().enumerate() {
        let Some(Sel::Face { point, .. }) = input.items.first() else {
            continue;
        };
        let Some(at) = proj.to_screen(*point) else {
            continue;
        };
        let color = if cmd == "mesh.align_landmarks" && i % 2 == 1 {
            egui::Color32::from_rgb(100, 212, 255)
        } else {
            solidcraft_theme::Tokens::get().sel_edge
        };
        painter.circle_filled(at, 4., color);
        painter.circle_stroke(at, 4., egui::Stroke::new(1., egui::Color32::BLACK));
        painter.text(at + egui::vec2(7., -5.), egui::Align2::LEFT_BOTTOM, input.label, egui::FontId::proportional(13.), color);
    }
}
use crate::{
    SolveApp,
    dialogs::{Dialog, Kind},
};
use solidcore_record::Vec3;
use solidcraft_engine::{Session, geom::prelude::*};

fn set_plane(s: &Session, inputs: &mut [SelInput], extra: &mut serde_json::Map<String, Value>, plane: u64) {
    extra.insert("alignment_plane".into(), json!(plane));
    if let Some(name) = s.fitted_plane_scan(plane) {
        inputs[0].items = vec![Sel::Body { name }];
    } else if inputs[0].items.is_empty() {
        let world = crate::open_meshing::session_world(s);
        let mut scans = world.bodies.iter().filter(|b| b.body.is_mesh());
        if let Some(b) = scans.next()
            && scans.next().is_none()
        {
            inputs[0].items.push(Sel::Body { name: b.name.clone() });
        }
    }
}
pub(crate) fn init_plane(s: &Session, d: &mut Dialog) {
    let planes = solidcraft_engine::view::construction_planes(s);
    let picked = s.selection.iter().find_map(|sel| {
        planes.iter().find(|(id, name, _)| match sel {
            Sel::Plane { name: pick } => pick == name,
            Sel::Feature { id: pick } => pick == id,
            _ => false,
        })
    });
    if let Some((id, _, _)) = picked.or_else(|| planes.last()) {
        set_plane(s, &mut d.inputs, &mut d.extra, *id);
    }
}
pub(crate) fn start_plane(app: &mut SolveApp, plane: u64) {
    app.start("mesh.align_plane_to_csys");
    if let Some(d) = app.dialog.as_mut()
        && matches!(d.kind, Kind::MeshEdit { command: "mesh.align_plane_to_csys", .. })
    {
        set_plane(&app.session, &mut d.inputs, &mut d.extra, plane);
    }
}
pub(crate) fn plane_rows(
    app: &SolveApp,
    ui: &mut egui::Ui,
    extra: &mut serde_json::Map<String, Value>,
    inputs: &mut [SelInput],
    target: &mut usize,
    side: &mut usize,
    angle: &mut String,
    delta: &mut [String; 3],
) -> bool {
    let planes = solidcraft_engine::view::construction_planes(&app.session);
    if planes.is_empty() {
        ui.label("Create a construction plane or use Fit Region Plane on a surface patch.");
    } else {
        let selected = extra.get("alignment_plane").and_then(Value::as_u64);
        let mut idx = planes.iter().position(|p| Some(p.0) == selected).unwrap_or(0);
        let names: Vec<_> = planes.iter().map(|p| p.1.as_str()).collect();
        crate::ui_kit::choice_row(ui, "Construction plane", "alignment_plane", &mut idx, &names);
        if Some(planes[idx].0) != selected {
            set_plane(&app.session, inputs, extra, planes[idx].0);
        }
    }
    crate::ui_kit::row(ui, "Target plane", |ui| {
        ui.horizontal(|ui| {
            for (i, label) in ["XY", "XZ", "YZ"].iter().enumerate() {
                let r = ui.selectable_value(target, i, *label);
                solidcraft_ui_registry::publish_handle(&format!("alignment:{label}"), r.rect.center());
            }
        });
    });
    let mut flip = *side != 0;
    let r = ui.checkbox(&mut flip, "Flip normal");
    *side = usize::from(flip);
    solidcraft_ui_registry::publish_handle("alignment:Flip normal", r.rect.center());
    let mut enter = crate::ui_kit::expr_row(ui, "In-plane angle", angle);
    enter |= crate::ui_kit::expr_row(ui, solidcraft_ui_text::tr("ui.scan_align.offset_u"), &mut delta[0]);
    enter |= crate::ui_kit::expr_row(ui, solidcraft_ui_text::tr("ui.scan_align.offset_v"), &mut delta[1]);
    ui.label("Drag the ring to rotate freely. Hold Ctrl to snap to 5°.");
    ui.add(egui::Label::new(solidcraft_ui_text::tr("ui.scan_align.offset_help")).wrap());
    enter
}
const TARGETS: [&str; 3] = ["xy", "xz", "yz"];
pub(crate) fn angle_deg(s: &Session, value: &str) -> Result<f64, String> {
    crate::length_input::eval(&s.doc, value, solidcraft_engine::doc::expr::Kind::Angle).map(|a| a.to_degrees()).map_err(|e| e.to_string())
}
fn settings(d: &Dialog) -> Option<(&str, u64, usize, bool, &str)> {
    let Kind::MeshEdit { command: "mesh.align_plane_to_csys", plane, side, ref amount, .. } = d.kind else { return None };
    Some((crate::mesh_lasso::body(&d.inputs)?, d.extra.get("alignment_plane")?.as_u64()?, plane, side != 0, amount))
}
pub(crate) fn preview(app: &SolveApp) -> Option<(String, solidcraft_engine::doc::Mat)> {
    let (name, id, target, flip, angle) = settings(app.dialog.as_ref()?)?;
    let (delta, _, _) = app
        .session
        .preview_mesh_plane_alignment_offset(id, TARGETS[target], flip, angle_deg(&app.session, angle).ok()?, dialog_offset(app).ok()?)
        .ok()?;
    Some((name.into(), delta))
}
fn ring(app: &SolveApp, d: &Dialog, proj: &Proj) -> Option<(Vec3, Vec3, f64, f64)> {
    let (_, id, target, flip, angle) = settings(d)?;
    let angle = angle_deg(&app.session, angle).ok()?;
    let (_, center, axis) = app
        .session
        .preview_mesh_plane_alignment_offset(
            id,
            TARGETS[target],
            flip,
            angle,
            match &d.kind {
                Kind::MeshEdit { delta, .. } => offset(&app.session, delta).ok()?,
                _ => [0., 0.],
            },
        )
        .ok()?;
    let radius = crate::gizmo::ring_radius(proj, center)?;
    Some((center, axis, radius, angle.to_radians()))
}
/// Screen ellipse of the clocking control. Keep a narrow but usable minor axis
/// when the geometric circle projects edge-on.
struct Dial {
    center: egui::Pos2,
    x: egui::Vec2,
    y: egui::Vec2,
}
impl Dial {
    fn projected(proj: &Proj, c: Vec3, n: Vec3, r: f64) -> Option<Self> {
        let center = proj.to_screen(c)?;
        let u = n.any_perp();
        let mut x = proj.to_screen(c + u * r)? - center;
        let mut y = proj.to_screen(c + n.cross(u) * r)? - center;
        let det = x.x * y.y - x.y * y.x;
        let sign = if det < 0. { -1. } else { 1. };
        let major = x.length().max(y.length());
        if major < 4. {
            return None;
        }
        if det.abs() / major < 13. {
            if x.length() >= y.length() {
                let a = x.normalized();
                y = a * y.dot(a) + egui::vec2(-a.y, a.x) * (13. * sign);
            } else {
                let b = y.normalized();
                x = b * x.dot(b) + egui::vec2(b.y, -b.x) * (13. * sign);
            }
        }
        Some(Self { center, x, y })
    }
    fn point(&self, angle: f64) -> egui::Pos2 {
        self.center + self.x * angle.cos() as f32 + self.y * angle.sin() as f32
    }
    fn angle(&self, p: egui::Pos2) -> f64 {
        let v = p - self.center;
        let det = self.x.x * self.y.y - self.x.y * self.y.x;
        let x = (v.x * self.y.y - v.y * self.y.x) / det;
        let y = (self.x.x * v.y - self.x.y * v.x) / det;
        f64::from(y.atan2(x))
    }
    fn hit(&self, p: egui::Pos2) -> bool {
        // Angle inversion is sensitive to pixel offsets near an edge-on ellipse.
        // Put the pick tolerance in pixels instead of normalized ellipse radii.
        (0..=72).any(|i| self.point(std::f64::consts::TAU * f64::from(i) / 72.).distance(p) <= 10.)
    }
}
pub(crate) fn owns_pointer(app: &SolveApp, proj: &Proj, p: egui::Pos2) -> bool {
    if app.dialog.as_ref().is_some_and(|d| translation_handles(app, d, proj).iter().any(|(_, a, b, _)| segment_distance(p, *a, *b) < 9.)) {
        return true;
    }
    app.dialog.as_ref().and_then(|d| ring(app, d, proj)).and_then(|(c, n, r, _)| Dial::projected(proj, c, n, r)).is_some_and(|dial| dial.hit(p))
}

pub(crate) fn draw_ring(app: &SolveApp, d: &mut Dialog, ui: &mut egui::Ui, painter: &egui::Painter, proj: &Proj) {
    draw_translation(app, d, ui, painter, proj);
    let Some((c, n, r, a)) = ring(app, d, proj) else {
        return;
    };
    let Some(dial) = Dial::projected(proj, c, n, r) else {
        return;
    };
    let full = std::f64::consts::TAU;
    let points = (0..=80).map(|i| dial.point(f64::from(i) * full / 80.)).collect();
    let stroke = egui::Stroke::new(2., solidcraft_theme::Tokens::get().manipulator);
    painter.add(egui::Shape::line(points, stroke));
    if let Some(z) = proj.to_screen(c + n * r * 0.8) {
        painter.arrow(dial.center, z - dial.center, stroke);
    }
    let knob = dial.point(a);
    painter.circle(knob, 6., stroke.color, egui::Stroke::new(1.5, egui::Color32::WHITE));
    solidcraft_ui_registry::publish_handle("alignment:ring", knob);
    for deg in [0, 30, 45, 60, 90, 120, 180, 270] {
        solidcraft_ui_registry::publish_handle(&format!("alignment:ring:{deg}"), dial.point(f64::from(deg).to_radians()));
    }
    let id = egui::Id::unique("alignment_plane_dial");
    let Some(p) = ui.input(|i| i.pointer.interact_pos()) else {
        return;
    };
    let stored = ui.data(|m| m.get_temp::<(f64, f64)>(id));
    let over = ui.clip_rect().intersect(proj.rect).contains(p) && ui.ctx().layer_id_at(p) == Some(ui.layer_id()) && dial.hit(p);
    if !over && stored.is_none() {
        return;
    }
    ui.ctx().set_cursor_icon(if stored.is_some() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
    let response = ui.interact(egui::Rect::from_center_size(p, egui::vec2(24., 24.)), id.with("hit"), egui::Sense::drag());
    if !response.dragged() && !response.drag_started() && !response.drag_stopped() {
        if !ui.input(|i| i.pointer.primary_down()) {
            ui.data_mut(|m| m.remove::<(f64, f64)>(id));
        }
        return;
    }
    let release = ui.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, .. } => Some(*pos),
            _ => None,
        })
    });
    let cursor = dial.angle(release.unwrap_or(p));
    let (last, angle) = stored.unwrap_or_else(|| (dial.angle(ui.input(|i| i.pointer.press_origin()).unwrap_or(p)), a));
    let delta = (cursor - last + std::f64::consts::PI).rem_euclid(full) - std::f64::consts::PI;
    let angle = angle + delta;
    let mut degrees = angle.to_degrees();
    if ui.input(|i| i.modifiers.ctrl && !i.modifiers.shift) {
        degrees = (degrees / 5.).round() * 5.;
    }
    if let Kind::MeshEdit { amount, .. } = &mut d.kind {
        *amount = format!("{degrees:.2} deg");
    }
    crate::tool_modifiers::set_manipulator_dragging(true);
    if release.is_some() || response.drag_stopped() {
        ui.data_mut(|m| m.remove::<(f64, f64)>(id));
    } else {
        ui.data_mut(|m| m.insert_temp(id, (cursor, angle)));
    }
}
pub(crate) fn offset(s: &Session, values: &[String; 3]) -> Result<[f64; 2], String> {
    let value = |i: usize| crate::length_input::eval(&s.doc, &values[i], solidcraft_engine::doc::expr::Kind::Length).map_err(|e| e.to_string());
    let out = [value(0)?, value(1)?];
    if out.iter().any(|v| !v.is_finite()) {
        return Err(solidcraft_ui_text::tr("ui.scan_align.invalid_vector").into());
    }
    Ok(out)
}
fn dialog_offset(app: &SolveApp) -> Result<[f64; 2], String> {
    match app.dialog.as_ref().map(|d| &d.kind) {
        Some(Kind::MeshEdit { delta, .. }) => offset(&app.session, delta),
        _ => Ok([0., 0.]),
    }
}
pub(crate) fn symmetry_rows(
    app: &SolveApp,
    ui: &mut egui::Ui,
    inputs: &mut Vec<SelInput>,
    extra: &mut serde_json::Map<String, Value>,
    mode: &mut usize,
    origin: &mut [String; 3],
    active: &mut usize,
) -> bool {
    use solidcraft_ui_text::tr;
    crate::ui_kit::choice_row(ui, tr("ui.scan_align.method"), "symmetry_method", mode, &[tr("ui.scan_align.explicit"), tr("ui.scan_align.pairs")]);
    let count = if *mode == 0 { 1 } else { 6 };
    if inputs.len() != count {
        *active = 0;
        let body = crate::mesh_lasso::body(inputs).map(str::to_owned);
        *inputs = if *mode == 0 {
            vec![SelInput::new("Mesh body", crate::selection::BODIES, false)]
        } else {
            ["Left 1", "Right 1", "Left 2", "Right 2", "Left 3", "Right 3"]
                .iter()
                .map(|s| SelInput::new(s, crate::selection::FACES, false).places())
                .collect()
        };
        if *mode == 0
            && let (Some(body), Some(input)) = (body, inputs.first_mut())
        {
            input.items = vec![Sel::Body { name: body }];
        }
    }
    let mut enter = false;
    if *mode == 0 {
        for (i, label) in ["ui.scan_align.origin_x", "ui.scan_align.origin_y", "ui.scan_align.origin_z"].iter().enumerate() {
            enter |= crate::ui_kit::expr_row(ui, tr(label), &mut origin[i]);
        }
        let mut normal: [String; 3] = std::array::from_fn(|i| {
            extra.get("symmetry_normal").and_then(|v| v.get(i)).and_then(Value::as_str).unwrap_or(if i == 0 { "1" } else { "0" }).into()
        });
        for (i, label) in ["ui.scan_align.normal_x", "ui.scan_align.normal_y", "ui.scan_align.normal_z"].iter().enumerate() {
            enter |= crate::ui_kit::expr_row(ui, tr(label), &mut normal[i]);
        }
        extra.insert("symmetry_normal".into(), json!(normal));
    } else {
        ui.add(egui::Label::new(tr("ui.scan_align.pair_help")).wrap());
    }
    ui.add(egui::Label::new(tr("ui.scan_align.workflow_help")).wrap());
    let _ = app;
    enter
}
pub(crate) fn symmetry_params(app: &SolveApp, d: &Dialog) -> Result<Value, String> {
    use solidcraft_ui_text::tr;
    let Kind::MeshEdit { side, delta, .. } = &d.kind else {
        return Err(tr("ui.scan_align.invalid_vector").into());
    };
    let body = crate::mesh_lasso::body(&d.inputs).ok_or_else(|| tr("ui.scan_align.pick_scan").to_string())?;
    if *side != 0 {
        let mut points = Vec::new();
        for input in &d.inputs {
            let Some(Sel::Face { body: name, point, .. }) = input.items.first() else {
                return Err(tr("ui.scan_align.complete_pairs").into());
            };
            if name != body {
                return Err(tr("ui.scan_align.same_scan").into());
            }
            points.push([point.x, point.y, point.z]);
        }
        if points.len() != 6 {
            return Err(tr("ui.scan_align.complete_pairs").into());
        }
        return Ok(json!({"body":body,"pairs":[[points[0],points[1]],[points[2],points[3]],[points[4],points[5]]]}));
    }
    let mut origin = [0.; 3];
    let mut normal = [0.; 3];
    for i in 0..3 {
        origin[i] = crate::length_input::eval(&app.session.doc, &delta[i], solidcraft_engine::doc::expr::Kind::Length).map_err(|e| e.to_string())?;
        let expr = d.extra.get("symmetry_normal").and_then(|v| v.get(i)).and_then(Value::as_str).unwrap_or(if i == 0 { "1" } else { "0" });
        normal[i] = app.session.doc.eval(expr, solidcraft_engine::doc::expr::Kind::Unitless).map_err(|e| e.to_string())?;
    }
    if origin.iter().chain(normal.iter()).any(|v| !v.is_finite()) || normal.iter().map(|v| v * v).sum::<f64>() < 1e-20 {
        return Err(tr("ui.scan_align.invalid_vector").into());
    }
    Ok(json!({"body":body,"origin":origin,"normal":normal}))
}
fn segment_distance(p: egui::Pos2, a: egui::Pos2, b: egui::Pos2) -> f32 {
    let v = b - a;
    let t = ((p - a).dot(v) / v.length_sq().max(1e-8)).clamp(0., 1.);
    p.distance(a + v * t)
}
fn translation_handles(app: &SolveApp, d: &Dialog, proj: &Proj) -> Vec<(usize, egui::Pos2, egui::Pos2, f64)> {
    let Some((c, _, r, _)) = ring(app, d, proj) else { return vec![] };
    let Kind::MeshEdit { plane, .. } = d.kind else { return vec![] };
    let axes = match plane {
        0 => [Vec3::X, Vec3::Y],
        1 => [Vec3::X, Vec3::Z],
        _ => [Vec3::Y, Vec3::Z],
    };
    axes.iter()
        .enumerate()
        .filter_map(|(i, axis)| {
            let a = proj.to_screen(c)?;
            let b = proj.to_screen(c + *axis * r * 0.7)?;
            ((b - a).length() > 12.).then_some((i, a, b, r * 0.7))
        })
        .collect()
}
fn draw_translation(app: &SolveApp, d: &mut Dialog, ui: &mut egui::Ui, painter: &egui::Painter, proj: &Proj) {
    let id = egui::Id::unique("scan_plane_translation");
    let Some(p) = ui.input(|i| i.pointer.interact_pos()) else { return };
    let mut state = ui.data(|m| m.get_temp::<(usize, egui::Pos2, egui::Vec2, f64, f64)>(id));
    for (axis, a, b, world_length) in translation_handles(app, d, proj) {
        let stroke = egui::Stroke::new(2.5, solidcraft_theme::Tokens::get().manipulator);
        painter.arrow(a, b - a, stroke);
        painter.text(b, egui::Align2::LEFT_BOTTOM, if axis == 0 { "U" } else { "V" }, egui::FontId::proportional(13.), stroke.color);
        solidcraft_ui_registry::publish_handle(if axis == 0 { "alignment:translate_u" } else { "alignment:translate_v" }, a + (b - a) * 0.65);
        let over = ui.clip_rect().intersect(proj.rect).contains(p)
            && ui.ctx().layer_id_at(p) == Some(ui.layer_id())
            && segment_distance(p, a, b) < 9.
            && p.distance(a) > 12.;
        if over && state.is_none() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            let response = ui.interact(egui::Rect::from_center_size(p, egui::vec2(20., 20.)), id.with(axis), egui::Sense::drag());
            if response.drag_started() {
                if let Kind::MeshEdit { delta, .. } = &d.kind
                    && let Ok(values) = offset(&app.session, delta)
                {
                    state = Some((axis, ui.input(|i| i.pointer.press_origin()).unwrap_or(p), b - a, world_length, values[axis]));
                }
            }
        }
    }
    if let Some((axis, start, screen, world, initial)) = state {
        let distance = f64::from((p - start).dot(screen) / screen.length_sq().max(1e-8)) * world;
        if let Kind::MeshEdit { delta, .. } = &mut d.kind {
            delta[axis] = format!("{} mm", initial + distance);
        }
        crate::tool_modifiers::set_manipulator_dragging(true);
        if ui.input(|i| i.pointer.primary_down()) {
            ui.data_mut(|m| m.insert_temp(id, (axis, start, screen, world, initial)));
        } else {
            ui.data_mut(|m| m.remove::<(usize, egui::Pos2, egui::Vec2, f64, f64)>(id));
        }
    }
}
