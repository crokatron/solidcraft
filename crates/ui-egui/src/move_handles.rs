//! Shared editable coordinate frame for body and component move handles.
use crate::{SolveApp, gizmo, ui_kit};
use solidcore_record::Vec3;
use solidcraft_engine::doc::{IDENTITY, Mat};
use solidcraft_engine::geom::prelude::*;

pub fn axes(frame: &Mat) -> [Vec3; 3] {
    [0, 1, 2].map(|i| Vec3::new(frame[i][0], frame[i][1], frame[i][2]))
}
pub fn origin(frame: &Mat) -> Vec3 {
    Vec3::new(frame[3][0], frame[3][1], frame[3][2])
}
pub fn at(origin: Vec3) -> Mat {
    let mut m = IDENTITY;
    m[3] = [origin.x, origin.y, origin.z, 1.];
    m
}
pub fn vector(frame: &Mat, local: Vec3) -> Vec3 {
    let a = axes(frame);
    a[0] * local.x + a[1] * local.y + a[2] * local.z
}
pub fn local(frame: &Mat, world: Vec3) -> Vec3 {
    let a = axes(frame);
    Vec3::new(world.dot(a[0]), world.dot(a[1]), world.dot(a[2]))
}

/// Rotate the handle orientation about one of its current axes, keeping its origin.
pub fn turn(frame: &Mat, axis: usize, radians: f64) -> Mat {
    let Some(direction) = axes(frame).get(axis).copied() else { return *frame };
    let rotation = gizmo::rotation(direction, radians);
    let mut result = *frame;
    for (i, dir) in axes(frame).iter().enumerate() {
        let v = gizmo::apply(&rotation, *dir);
        result[i] = [v.x, v.y, v.z, 0.];
    }
    result
}

fn euler(frame: &Mat) -> [f64; 3] {
    let pitch = (-frame[0][2]).clamp(-1., 1.).asin();
    if pitch.cos().abs() > 1e-8 {
        [frame[1][2].atan2(frame[2][2]), pitch, frame[0][1].atan2(frame[0][0])]
    } else {
        [0., pitch, (-frame[1][0]).atan2(frame[1][1])]
    }
}
fn from_euler(origin: Vec3, a: [f64; 3]) -> Mat {
    let r = gizmo::mul(&gizmo::mul(&gizmo::rotation(Vec3::Z, a[2]), &gizmo::rotation(Vec3::Y, a[1])), &gizmo::rotation(Vec3::X, a[0]));
    let mut m = at(origin);
    for c in 0..3 {
        for row in 0..3 {
            m[c][row] = r[row][c];
        }
    }
    m
}

fn reference_row(app: &SolveApp, ui: &mut egui::Ui, key: &mut String, label: &str, id: &str) {
    if key.is_empty() {
        *key = "world".into();
    }
    let frames = app.session.coordinate_frames();
    let known = frames.iter().position(|f| f.key == *key);
    let mut labels: Vec<String> = frames.iter().map(|f| f.label.clone()).collect();
    let mut selected = known.unwrap_or(labels.len());
    if known.is_none() {
        labels.push(solidcraft_ui_text::tr("move.missing_csys").to_string());
    }
    let options: Vec<&str> = labels.iter().map(String::as_str).collect();
    let response = ui_kit::choice_row(ui, label, id, &mut selected, &options);
    crate::scenario::publish_handle(&format!("move-handles:{id}"), response.rect.center());
    if let Some(preset) = frames.get(selected) {
        key.clone_from(&preset.key);
    }
}

/// Numeric handle pose, plus a named CSYS preset. Changes affect only the caller's draft frame.
pub fn rows(app: &SolveApp, ui: &mut egui::Ui, frame: &mut Mat, reference: &mut String) -> bool {
    let before = *frame;
    reference_row(app, ui, reference, solidcraft_ui_text::tr("move.handles_reference"), "source-csys");
    let set = ui.button(solidcraft_ui_text::tr("move.handles_from_reference"));
    crate::scenario::publish_handle("move-handles:from-reference", set.rect.center());
    if set.clicked() {
        if let Ok(m) = app.session.coordinate_frame(reference) {
            *frame = m;
        }
    }
    *frame != before
}

/// Independent pose dock, shown by a caller outside its main feature container.
pub fn pose_dock(app: &SolveApp, ctx: &egui::Context, owner: &str, frame: &mut Mat) -> (bool, Option<egui::Rect>) {
    let bounds = crate::agent_chat::tool_space(app, crate::shell::content_rect(app, app.viewport.rect.unwrap_or_else(|| ctx.content_rect())));
    if !bounds.is_finite() || !bounds.is_positive() {
        return (false, None);
    }
    let t = solidcraft_theme::Tokens::get();
    let at = egui::pos2(bounds.right() - t.space_8, bounds.top() + t.space_8);
    let mut rect = None;
    let mut changed = false;
    ui_kit::Window::new(egui::Id::unique(("move-pose-dock", owner)), "move", solidcraft_ui_text::tr("move.pose_title"))
        .placed(egui::Align2::RIGHT_TOP, at)
        .cards(2.0)
        .show_passive_body(ctx, |ui| {
            ui.add_enabled_ui(app.applying.is_none(), |ui| {
                changed = pose_fields(app, ui, frame);
            });
            let body = ui.min_rect();
            rect = Some(egui::Rect::from_min_max(
                body.min - egui::vec2(t.space_12, t.form_header_height + t.space_12),
                body.max + egui::vec2(t.space_12, t.space_12),
            ));
        });
    if let Some(rect) = rect {
        crate::scenario::publish_rect("move-handles:pose-dock", rect);
    }
    (changed, rect)
}

/// Compact editable origin/orientation for a shared dock or inline move form.
/// The frame changes only after a finite, bounded pose edit.
pub fn pose_fields(app: &SolveApp, ui: &mut egui::Ui, frame: &mut Mat) -> bool {
    let before = *frame;
    let mut o = [frame[3][0], frame[3][1], frame[3][2]];
    let mut angles = euler(frame).map(f64::to_degrees);
    let previous = (o, angles);
    ui_kit::row(ui, solidcraft_ui_text::tr("move.pose_origin"), |ui| {
        ui.horizontal(|ui| {
            for (i, axis) in ["X", "Y", "Z"].iter().enumerate() {
                ui.push_id(("origin", i), |ui| {
                    ui.label(*axis);
                    let response = ui_kit::length_field(ui, &mut o[i], &app.session.doc.units, app.preferences.length_decimals);
                    crate::scenario::publish_target(&format!("move-handles:origin:{i}"), response.rect);
                });
            }
        });
    });
    ui_kit::row(ui, solidcraft_ui_text::tr("move.pose_angles"), |ui| {
        ui.horizontal(|ui| {
            for (i, axis) in ["X", "Y", "Z"].iter().enumerate() {
                ui.push_id(("angle", i), |ui| {
                    ui.label(*axis);
                    let response = ui_kit::angle_field(ui, &mut angles[i]);
                    crate::scenario::publish_target(&format!("move-handles:angle:{i}"), response.rect);
                });
            }
        });
    });
    if previous != (o, angles) && o.iter().chain(&angles).all(|v| v.is_finite()) && o.iter().all(|v| v.abs() <= 1e9) {
        *frame = from_euler(Vec3::new(o[0], o[1], o[2]), angles.map(f64::to_radians));
    }
    *frame != before
}

/// Target CSYS picker and explicit align action. A missing old reference remains stale.
pub fn target_row(app: &SolveApp, ui: &mut egui::Ui, target: &mut String) -> bool {
    reference_row(app, ui, target, solidcraft_ui_text::tr("move.target_csys"), "target-csys");
    let button = ui.button(solidcraft_ui_text::tr("move.align_handles_to_csys"));
    crate::scenario::publish_handle("move-handles:align", button.rect.center());
    button.clicked()
}
