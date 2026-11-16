//! A local distance/angle next to the component manipulator, sharing the body move editor.
use crate::{SolveApp, browser::OccMove, gizmo, move_handles};
use solidcraft_engine::doc::{Mat, expr::Kind};

#[derive(Clone)]
struct Draft {
    owner: (u64, Vec<u64>),
    axis: usize,
    angle: bool,
    handles_only: bool,
    base: Mat,
    last_frame: Mat,
    text: String,
    focus: bool,
    last_seen: u64,
}
fn key() -> egui::Id {
    egui::Id::unique("occurrence-move-value-draft")
}
fn shown(app: &SolveApp, angle: bool, value: f64) -> String {
    if angle { format!("{value} deg") } else { crate::length_input::shown(&app.session.doc, value) }
}
pub(crate) fn select(app: &SolveApp, ctx: &egui::Context, m: &OccMove, handle: gizmo::Handle, focus: bool) {
    let (axis, angle) = match handle {
        gizmo::Handle::Axis(axis) => (axis, false),
        gizmo::Handle::Ring(axis) => (axis, true),
        _ => {
            ctx.data_mut(|d| d.remove::<Draft>(key()));
            return;
        }
    };
    let frame = crate::browser::occurrence_handle_frame(m);
    let value = if m.handles_only {
        0.
    } else if angle {
        m.angles[axis]
    } else {
        m.translate[axis]
    };
    let draft = Draft {
        owner: (m.occurrence, m.path.clone()),
        axis,
        angle,
        handles_only: m.handles_only,
        base: frame,
        last_frame: frame,
        text: shown(app, angle, value),
        focus,
        last_seen: ctx.cumulative_frame_nr(),
    };
    ctx.data_mut(|d| d.insert_temp(key(), draft));
}

/// The drag and its popup use the same incremental basis; editing after releasing must
/// replace the shown distance rather than applying it twice.
pub(crate) fn dragged(app: &SolveApp, ctx: &egui::Context, m: &OccMove, change: gizmo::Change) {
    let Some(mut draft) = ctx.data(|d| d.get_temp::<Draft>(key())) else { return };
    let value = if m.handles_only {
        match change {
            gizmo::Change::Translate(v) => {
                let local = move_handles::local(&draft.base, v);
                [local.x, local.y, local.z][draft.axis]
            }
            gizmo::Change::Rotate { angle, .. } => angle.to_degrees(),
        }
    } else if draft.angle {
        m.angles[draft.axis]
    } else {
        m.translate[draft.axis]
    };
    draft.text = shown(app, draft.angle, value);
    draft.last_frame = crate::browser::occurrence_handle_frame(m);
    draft.focus = false;
    ctx.data_mut(|d| d.insert_temp(key(), draft));
}

pub(crate) fn show(app: &mut SolveApp, ui: &mut egui::Ui, proj: &crate::viewport::Proj, m: &mut OccMove) {
    let Some(mut draft) = ui.ctx().data(|d| d.get_temp::<Draft>(key())) else { return };
    if draft.owner != (m.occurrence, m.path.clone()) || ui.ctx().cumulative_frame_nr().saturating_sub(draft.last_seen) > 3 {
        return;
    }
    draft.last_seen = ui.ctx().cumulative_frame_nr();
    let frame = crate::browser::occurrence_handle_frame(m);
    if draft.handles_only != m.handles_only || frame != draft.last_frame {
        // Snapping or changing the independent pose dock starts a new handle offset.
        draft.base = frame;
        draft.last_frame = frame;
        draft.handles_only = m.handles_only;
        let value = if m.handles_only {
            0.
        } else if draft.angle {
            m.angles[draft.axis]
        } else {
            m.translate[draft.axis]
        };
        draft.text = shown(app, draft.angle, value);
    }
    let t = gizmo::Triad { center: move_handles::origin(&frame), axes: move_handles::axes(&frame), translate: true, rotate: true };
    let Some(center) = proj.to_screen(t.center) else { return };
    let mut place = crate::canvas::box_place(app, proj);
    // Component movement has no regular Dialog, so its windows are not represented
    // by viewport.dialog_rect. Respect their actual rendered areas when placing this field.
    for id in [egui::Id::unique("sc_occ_move"), egui::Id::unique(("move-pose-dock", "component"))] {
        if let Some(rect) = ui.ctx().memory(|memory| memory.area_rect(id)) {
            place.dialog = Some(place.dialog.map_or(rect, |other| other.union(rect)));
        }
    }
    place.handles = gizmo::handles_region(proj, &t);
    if let Some(markers) = crate::move_handle_snap::markers_region(ui.ctx(), proj) {
        place.handles = Some(place.handles.map_or(markers, |rect| rect.union(markers)));
    }
    let labels = if draft.angle { ["X angle", "Y angle", "Z angle"] } else { ["X distance", "Y distance", "Z distance"] };
    let label = solidcraft_ui_text::tr_label(labels[draft.axis]);
    let old = draft.text.clone();
    let field = crate::canvas::value_box_in(
        ui,
        (egui::Id::unique("occurrence-move-value-area"), egui::Id::unique("occurrence-move-value-input")),
        &label,
        &mut draft.text,
        (center + egui::vec2(gizmo::LEN_PX as f32 + 16., -12.), egui::Align2::LEFT_TOP),
        draft.focus,
        false,
        place,
    );
    draft.focus = false;
    if let Some(rect) = field {
        crate::scenario::publish_handle("occurrence-move:canvas-value", rect.center());
    }
    if old != draft.text {
        let kind = if draft.angle { Kind::Angle } else { Kind::Length };
        let value = crate::length_input::no_definition(&draft.text, &label)
            .ok()
            .and_then(|_| crate::length_input::eval(&app.session.doc, &draft.text, kind).ok());
        if let Some(value) = value.filter(|v| v.is_finite() && v.abs() <= 1e9) {
            if m.handles_only {
                let next = if draft.angle {
                    move_handles::turn(&draft.base, draft.axis, value)
                } else {
                    let direction = move_handles::axes(&draft.base)[draft.axis] * value;
                    let mut next = draft.base;
                    next[3][0] += direction.x;
                    next[3][1] += direction.y;
                    next[3][2] += direction.z;
                    next
                };
                m.handle_frame = Some(next);
                m.pivot = Some(move_handles::origin(&next));
            } else {
                if draft.angle {
                    m.angles[draft.axis] = value.to_degrees();
                } else {
                    m.translate[draft.axis] = value;
                }
                crate::browser::apply_occurrence_move(app, m);
            }
            draft.last_frame = crate::browser::occurrence_handle_frame(m);
        }
    }
    ui.ctx().data_mut(|d| d.insert_temp(key(), draft));
}
