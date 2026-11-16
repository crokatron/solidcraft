//! Review user shortcut interoperability before changing the active keymap.
use crate::{SolveApp, nx_shortcuts, ui_kit};

#[derive(Default)]
pub(crate) struct State {
    report: Option<nx_shortcuts::Report>,
    source: String,
    replace: bool,
    backup: Option<serde_json::Value>,
}

pub(crate) fn import(app: &mut SolveApp, source: &str, text: &str) -> Result<(), String> {
    if text.trim_start_matches('\u{feff}').trim_start().starts_with('<') {
        let report = nx_shortcuts::parse(text.trim_start_matches('\u{feff}'))?;
        app.keymap.nx.report = Some(report);
        app.keymap.nx.source = source.to_owned();
        app.keymap.nx.replace = false;
        Ok(())
    } else {
        let value = serde_json::from_str(text).map_err(|e| format!("Invalid shortcut profile: {e}"))?;
        crate::toolbar_custom::import(app, &value)
    }
}

pub(crate) fn show(app: &mut SolveApp, ui: &mut egui::Ui) {
    ui_kit::text(ui, app.tr("nx_shortcuts.import_hint"), ui_kit::Text::Hint("nx_shortcuts_hint"));
    let mut state = std::mem::take(&mut app.keymap.nx);
    if let Some(report) = &mut state.report {
        ui.collapsing(app.tr("nx_shortcuts.report"), |ui| {
            ui.label(&state.source);
            ui_kit::text(ui, app.tr("nx_shortcuts.limit"), ui_kit::Text::Note("nx_shortcuts_limit"));
            if !app.keymap.read_only {
                ui_kit::checkbox(ui, app.tr("nx_shortcuts.replace"), &mut state.replace);
                ui.horizontal_wrapped(|ui| {
                    let apply = ui_kit::button(ui, app.tr("nx_shortcuts.apply"), ui_kit::Button::Primary);
                    crate::scenario::publish_target("keymap:nx_apply", apply.rect);
                    if apply.clicked() {
                        if state.backup.is_none() {
                            state.backup = Some(crate::toolbar_custom::export(app));
                        }
                        nx_shortcuts::apply(app, report, state.replace);
                    }
                    if state.backup.is_some()
                        && ui_kit::button(ui, app.tr("nx_shortcuts.restore"), ui_kit::Button::Normal).clicked()
                        && let Some(backup) = state.backup.take()
                    {
                        let _ = crate::toolbar_custom::import(app, &backup);
                        for entry in &mut report.entries {
                            if entry.status == nx_shortcuts::Status::Applied {
                                entry.status = nx_shortcuts::Status::Ready;
                            }
                        }
                    }
                    if ui_kit::button(ui, app.tr("nx_shortcuts.save_report"), ui_kit::Button::Normal).clicked()
                        && let Ok(text) = serde_json::to_string_pretty(report)
                    {
                        app.pick_save("NX shortcut import report.json", &["json"], move |app, path| {
                            if let Some(path) = path
                                && let Err(error) = solidcraft_engine::io::vfs::write(&path, text.as_bytes())
                            {
                                app.keymap.message = Some(error.to_string());
                            }
                        });
                    }
                });
            }
            for entry in &report.entries {
                let status = match entry.status {
                    nx_shortcuts::Status::Ready => "nx_shortcuts.ready",
                    nx_shortcuts::Status::Applied => "nx_shortcuts.applied",
                    nx_shortcuts::Status::Unsupported => "nx_shortcuts.unsupported",
                    nx_shortcuts::Status::Suppressed => "nx_shortcuts.suppressed",
                    nx_shortcuts::Status::Conflict => "nx_shortcuts.conflict",
                };
                ui.label(format!("{} · {} · {} → {}", app.tr(status), entry.key, entry.nx_command, entry.command_id.as_deref().unwrap_or("—")));
                ui_kit::text(ui, &format!("{} · {}", entry.profile, entry.note), ui_kit::Text::Hint("nx_binding_note"));
            }
        });
    }
    app.keymap.nx = state;
}

/// Same import/review path used by the file picker, for automation without dialogs.
pub(crate) fn control(app: &mut SolveApp, text: Option<&str>, source: &str, apply: bool, replace: bool) -> Result<serde_json::Value, String> {
    if let Some(text) = text {
        import(app, source, text)?;
    }
    let mut state = std::mem::take(&mut app.keymap.nx);
    if apply && let Some(report) = &mut state.report {
        if state.backup.is_none() {
            state.backup = Some(crate::toolbar_custom::export(app));
        }
        nx_shortcuts::apply(app, report, replace);
    }
    let result = serde_json::json!({"source": state.source, "report": state.report});
    app.keymap.nx = state;
    Ok(result)
}
