//! The two prompts the Browser opens over the viewport: Move/Copy of an occurrence (offsets and
//! turns, committed with scoped OK/Apply) and the Capture Position question asked before a
//! command or a save while components are moved but not captured.

use serde_json::json;

use crate::SolveApp;
use crate::browser::{answer_capture, apply_occurrence_move, occurrence_origin, panel_pos, revert_occurrence_dialog};
use crate::ui_kit;

/// Move Component: draft handle pose is independent of pending occurrence placement.
pub(crate) fn occurrence_panel(app: &mut SolveApp, ui: &mut egui::Ui) {
    let Some(mut m) = app.tree.occurrence_move.clone() else {
        return;
    };
    if !m.path.is_empty()
        && let Err(error) = app.session.occurrence_placement(&m.path)
    {
        app.tree.occurrence_move = None;
        app.set_status(error.to_string(), true);
        return;
    }
    if m.baseline.is_none() {
        m.had_pending = app.session.pending_moves.contains_key(&m.occurrence);
        m.baseline = app
            .session
            .pending_moves
            .get(&m.occurrence)
            .copied()
            .or_else(|| app.session.doc.occurrences.iter().find(|o| o.id == m.occurrence).map(|o| o.transform));
        m.cancel_baseline = m.baseline;
        if app.preferences.remember_dialog_values
            && let Some(values) = app.preferences.command_dialog_values.get("occurrence.move")
        {
            m.handles_only = values["handles_only"].as_bool().unwrap_or(false);
            if let Some(target) = values["target_frame"].as_str() {
                m.target_frame = target.into();
            }
            if let Some(reference) = values["handle_reference"].as_str() {
                m.handle_reference = reference.into();
            }
        }
    }
    if m.pivot.is_none() {
        m.pivot = if m.path.is_empty() {
            occurrence_origin(app, m.occurrence)
        } else {
            app.session.occurrence_placement(&m.path).ok().map(|p| solidcraft_engine::doc::apply_point(&p.world, solidcore_record::Vec3::ZERO))
        };
    }
    if m.handle_frame.is_none() {
        m.handle_frame = m.pivot.map(crate::move_handles::at);
    }
    let ctx = ui.ctx().clone();
    let name = app.session.doc.occurrences.iter().find(|o| o.id == m.occurrence).map(|o| o.name.clone()).unwrap_or_default();
    let shared = !m.path.is_empty() && app.session.occurrence_scope_count(&m.path).is_ok_and(|n| n > 1);
    let title = solidcraft_ui_text::format("browser.move_title", &[("name", &name)]);
    let mut pose_dock = None;
    if m.handles_only {
        if let Some(frame) = &mut m.handle_frame {
            pose_dock = crate::move_handles::pose_dock(app, &ctx, "component", frame).1;
            m.pivot = Some(crate::move_handles::origin(frame));
        }
    }
    let (pivot, mut at) = crate::shell::floating_editor_anchor(app, &ctx).unwrap_or_else(|| (egui::Align2::LEFT_TOP, panel_pos(app)));
    if pivot == egui::Align2::RIGHT_TOP {
        if let Some(rect) = pose_dock {
            at.y = at.y.max(rect.bottom() + solidcraft_theme::Tokens::get().space_8);
        }
    }
    let (units, decimals) = (app.session.doc.units.clone(), app.preferences.length_decimals);
    let before = (m.translate, m.angles);
    let mut toggle = None;
    let mut align = false;
    let mut snapped_frame = None;
    let keyboard_submit = ui.input(|i| i.key_pressed(egui::Key::Enter));
    let window = ui_kit::Window::new(egui::Id::unique("sc_occ_move"), "move", &title);
    let window = if let Some(rect) = pose_dock {
        // A previously dragged editor must not cover the independent handle dock.
        let screen = ctx.content_rect();
        window.anchor(
            egui::Align2::RIGHT_TOP,
            egui::vec2(rect.right() - screen.right(), rect.bottom() + solidcraft_theme::Tokens::get().space_8 - screen.top()),
        )
    } else {
        window.placed(pivot, at)
    };
    let (closed, mut action) = window.fields().cards(2.0).actions("OK", "Cancel", Some("Apply")).show(&ctx, true, |ui| {
        if shared {
            ui_kit::text(
                ui,
                solidcraft_ui_text::tr_label("This child is shared. Position changes apply to every copy of its parent."),
                ui_kit::Text::Note("shared"),
            );
        }
        let mut only = m.handles_only;
        let check = ui.checkbox(&mut only, solidcraft_ui_text::tr("move.handles_only"));
        crate::scenario::publish_handle("occurrence-move:handles-only", check.rect.center());
        if only != m.handles_only {
            toggle = Some(only);
        }
        let mut frame = crate::browser::occurrence_handle_frame(&m);
        let owner = format!("component:{}:{:?}", m.occurrence, m.path);
        if crate::move_handle_snap::rows(app, ui, &owner, &mut frame) {
            if m.handles_only {
                m.handle_frame = Some(frame);
                m.pivot = Some(crate::move_handles::origin(&frame));
            } else {
                snapped_frame = Some(frame);
            }
        }
        if m.handles_only {
            ui_kit::text(ui, solidcraft_ui_text::tr("move.handles_only_hint"), ui_kit::Text::Hint("handles"));
            if let Some(frame) = &mut m.handle_frame {
                crate::move_handles::rows(app, ui, frame, &mut m.handle_reference);
                m.pivot = Some(crate::move_handles::origin(frame));
            }
        } else {
            ui_kit::section(ui, "occurrence-move-manual-values", solidcraft_ui_text::tr("move.manual_values"), false, |ui| {
                for (k, axis) in ["X distance", "Y distance", "Z distance"].into_iter().enumerate() {
                    let response =
                        ui_kit::row(ui, solidcraft_ui_text::tr_label(axis), |ui| ui_kit::length_field(ui, &mut m.translate[k], &units, decimals));
                    crate::scenario::publish_handle(&format!("occurrence-move:distance:{k}"), response.rect.center());
                }
                for (k, axis) in ["X angle", "Y angle", "Z angle"].into_iter().enumerate() {
                    let response = ui_kit::row(ui, solidcraft_ui_text::tr_label(axis), |ui| ui_kit::angle_field(ui, &mut m.angles[k]));
                    crate::scenario::publish_handle(&format!("occurrence-move:angle:{k}"), response.rect.center());
                }
            });
            align = crate::move_handles::target_row(app, ui, &mut m.target_frame);
        }
    });
    if before != (m.translate, m.angles) {
        apply_occurrence_move(app, &m);
    }
    if let Some(only) = toggle {
        crate::browser::rebase_occurrence_move(app, &mut m);
        m.handles_only = only;
    }
    if align || snapped_frame.is_some() {
        let source = crate::browser::occurrence_handle_frame(&m);
        match snapped_frame
            .map(Ok)
            .unwrap_or_else(|| app.session.coordinate_frame(&m.target_frame))
            .and_then(|target| solidcraft_engine::origins::frame_delta(&source, &target).map(|delta| (target, delta)))
        {
            Ok((target, delta)) => {
                let matrix: [[f64; 4]; 4] = std::array::from_fn(|r| std::array::from_fn(|c| delta[c][r]));
                let mut params = json!({"occurrence":m.occurrence,"space":"world","matrix":matrix,"capture":false});
                if !m.path.is_empty() {
                    params["path"] = json!(m.path);
                }
                if let Ok(result) = app.run("occurrence.move", params) {
                    if result["reverted"] != true {
                        crate::browser::rebase_occurrence_move(app, &mut m);
                        m.handle_frame = Some(target);
                        m.pivot = Some(crate::move_handles::origin(&target));
                    }
                }
            }
            Err(error) => app.set_status(error.to_string(), true),
        }
    }
    if app.preferences.remember_dialog_values
        && keyboard_submit
        && action == ui_kit::Action::Ok
        && app.preferences.command_dialog_values.get("occurrence.move").and_then(|v| v["confirmation"].as_str()) == Some("apply")
    {
        action = ui_kit::Action::Apply;
    }
    app.tree.occurrence_move = Some(m);
    if closed {
        confirm_occurrence_move(app, ui_kit::Action::Cancel);
    } else if action != ui_kit::Action::None {
        confirm_occurrence_move(app, action);
    }
}

/// Confirmation is scoped to this occurrence; other pending moves stay pending.
pub fn confirm_occurrence_move(app: &mut SolveApp, action: ui_kit::Action) {
    let Some(mut m) = app.tree.occurrence_move.clone() else {
        return;
    };
    if action == ui_kit::Action::None {
        return;
    }
    let word = match action {
        ui_kit::Action::Apply => "apply",
        ui_kit::Action::Cancel => "cancel",
        _ => "ok",
    };
    if app.preferences.remember_dialog_values {
        app.preferences.command_dialog_values.insert(
        "occurrence.move".into(),
        json!({"version":1,"confirmation":word,"handles_only":m.handles_only,"target_frame":m.target_frame,"handle_reference":m.handle_reference}),
    );
    }
    if action == ui_kit::Action::Cancel {
        revert_occurrence_dialog(app, &m);
        app.tree.occurrence_move = None;
        return;
    }
    if let Some(pending) = app.session.pending_moves.get(&m.occurrence).copied() {
        let matrix: [[f64; 4]; 4] = std::array::from_fn(|r| std::array::from_fn(|c| pending[c][r]));
        let mut params = json!({"occurrence":m.occurrence,"space":"local","to_matrix":matrix,"capture":true});
        if !m.path.is_empty() {
            params["path"] = json!(m.path);
        }
        match app.run("occurrence.move", params) {
            Ok(result) if result["reverted"] != true => {}
            _ => {
                app.tree.occurrence_move = Some(m);
                return;
            }
        }
    }
    if action == ui_kit::Action::Ok {
        app.tree.occurrence_move = None;
    } else {
        crate::browser::rebase_occurrence_move(app, &mut m);
        m.cancel_baseline = m.baseline;
        m.had_pending = false;
        app.tree.occurrence_move = Some(m);
    }
}

/// The Capture Position question: capture, revert or cancel the waiting command (or save).
pub(crate) fn capture_prompt(app: &mut SolveApp, ctx: &egui::Context) {
    if app.tree.capture_prompt.is_none() {
        return;
    }
    let n = app.session.pending_moves.len();
    let (closed, action) = ui_kit::Window::new(egui::Id::unique("sc_capture_prompt"), "capture", solidcraft_ui_text::tr_label("Capture Position"))
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, -80.0))
        .actions("Capture Position", "Cancel", Some("Revert"))
        .show(ctx, true, |ui| {
            let question = if n == 1 {
                solidcraft_ui_text::tr("browser.capture_one").to_string()
            } else {
                solidcraft_ui_text::format("browser.capture_many", &[("count", &n.to_string())])
            };
            ui_kit::text(ui, &question, ui_kit::Text::Report("question", false));
            ui_kit::text(ui, solidcraft_ui_text::tr("browser.capture_note"), ui_kit::Text::Note("capture"));
        });
    let answer = match action {
        ui_kit::Action::Ok => Some("capture"),
        ui_kit::Action::Apply => Some("revert"),
        ui_kit::Action::Cancel => Some("cancel"),
        ui_kit::Action::None if closed => Some("cancel"),
        ui_kit::Action::None => None,
    };
    if let Some(answer) = answer {
        let _ = answer_capture(app, answer);
    }
}
