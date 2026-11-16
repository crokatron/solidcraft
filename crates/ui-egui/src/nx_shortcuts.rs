//! Import user-authored NX accelerator configuration, without discarding unmapped records.
use quick_xml::events::{BytesStart, Event};
use serde::{Deserialize, Serialize};

use crate::{SolveApp, keymap};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub(crate) struct Report {
    pub entries: Vec<Entry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Entry {
    pub profile: String,
    pub nx_command: String,
    pub key: String,
    pub command_id: Option<String>,
    pub note: String,
    pub status: Status,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) enum Status {
    Ready,
    Applied,
    Unsupported,
    Suppressed,
    Conflict,
}

/// Parse only explicit accelerator records; ribbon buttons are not keyboard bindings.
/// DTDs are refused and resource limits also cover malformed/untrusted configuration.
pub(crate) fn parse(text: &str) -> Result<Report, String> {
    if text.len() > 16 * 1024 * 1024 {
        return Err("NX shortcut file exceeds 16 MiB".into());
    }
    let mut reader = quick_xml::Reader::from_str(text);
    let mut stack: Vec<(String, String)> = Vec::new();
    let mut report = Report::default();
    let mut root_seen = false;
    loop {
        let event = reader.read_event().map_err(|e| format!("Invalid NX shortcut XML: {e}"))?;
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let tag = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                if stack.is_empty() {
                    if root_seen || tag != "NX_PROFILES" {
                        return Err("Expected one NX_PROFILES root element".into());
                    }
                    root_seen = true;
                }
                let name = name_attribute(&e)?;
                if tag == "Accel" {
                    if stack.last().is_none_or(|(tag, _)| tag != "Accelerators") {
                        return Err("NX Accel is outside Accelerators".into());
                    }
                    let action = stack.iter().rev().find(|(tag, _)| tag == "Action").map(|(_, name)| name);
                    let Some(action) = action.filter(|name| !name.is_empty()) else {
                        return Err("NX accelerator is missing its action name".into());
                    };
                    if name.is_empty() {
                        return Err(format!("NX accelerator for {action} is missing its key"));
                    }
                    let profile = stack.iter().rev().find(|(tag, _)| tag == "Profile").map(|(_, name)| name.as_str()).unwrap_or("Global");
                    report.entries.push(entry(profile, action, &name));
                    if report.entries.len() > 10_000 {
                        return Err("NX shortcut file exceeds 10000 bindings".into());
                    }
                }
                if !empty {
                    stack.push((tag, name));
                    if stack.len() > 128 {
                        return Err("NX shortcut XML nesting exceeds 128 elements".into());
                    }
                }
            }
            Event::End(_) => {
                stack.pop();
            }
            Event::DocType(_) => return Err("NX shortcut XML must not contain a DTD".into()),
            Event::Eof => break,
            _ => {}
        }
    }
    if !root_seen || !stack.is_empty() {
        return Err("Incomplete NX shortcut XML".into());
    }
    Ok(report)
}

fn name_attribute(e: &BytesStart<'_>) -> Result<String, String> {
    let mut name = String::new();
    for attr in e.attributes() {
        let attr = attr.map_err(|e| format!("Invalid NX XML attribute: {e}"))?;
        if attr.key.as_ref() == b"name" {
            name =
                attr.normalized_value(quick_xml::XmlVersion::Implicit1_0).map_err(|e| format!("Invalid NX XML attribute value: {e}"))?.into_owned();
        }
    }
    Ok(name)
}

fn entry(profile: &str, action: &str, key: &str) -> Entry {
    let (mapped, note) = mapping(action);
    let suppressed = action == "NX_SUPPRESSED_KEYS_SECTION";
    let mut entry = Entry {
        profile: profile.into(),
        nx_command: action.into(),
        key: key.into(),
        command_id: mapped.map(str::to_owned),
        note: note.into(),
        status: if suppressed {
            Status::Suppressed
        } else if mapped.is_some() {
            Status::Ready
        } else {
            Status::Unsupported
        },
    };
    if let Some(id) = mapped {
        if solidcraft_engine::find_command(id).is_none() {
            entry.command_id = None;
            entry.status = Status::Unsupported;
            entry.note = format!("Command {id} is unavailable in this build");
        }
    }
    entry
}

fn mapping(action: &str) -> (Option<&'static str>, &'static str) {
    let id = match action {
        "UG_SKETCH_LINE" => "sketch.line",
        "UG_SKETCH_CIRCLE" => "sketch.circle.center",
        "UG_SKETCH_STUDIO_SPLINE" => "sketch.spline.fit_point",
        "UG_SKETCH_OFFSET_MOVE_CURVES" => "sketch.offset",
        "UG_SKETCH_ACTIVE_REFERENCE_SWITCH" => "sketch.construction",
        "UG_SKETCH_ADD_EXTRACTED_CURVES" => "sketch.project",
        "UG_SKETCH_LINEAR_DIMENSION" | "UG_SKETCH_RADIAL_DIMENSION" | "UG_SKETCH_ANGULAR_DIMENSION" => {
            return (Some("sketch.dimension"), "Sketch Dimension chooses its dimension type from selected geometry");
        }
        "UG_INFO_GEOMETRIC_VOLUME_OF_BODIES" | "UG_INFO_GEOMETRIC_MEASUREMENT" => "inspect.measure",
        "UG_SBSM_FLAT_PATTERN_FEATURE" => "sheet.flat_pattern",
        "UG_ASSY_INSERT_NEW_COMPONENT" => "component.create",
        "UG_MATERIAL_ASSIGN" => "material.assign",
        "UG_SBSM_CONVTOSM_FEATURE" => "sheet.convert",
        "UG_SBSM_PREFERENCES" => return (Some("sheet.manage_rules"), "Opens SolidCraft sheet metal rules"),
        "NX_SUPPRESSED_KEYS_SECTION" => return (None, "Disabled NX key; preserved for reference, does not disable SolidCraft or sketch bindings"),
        "UG_SKETCH_PERIMETER_DIM" => return (None, "No perimeter dimension tool is available"),
        "UG_LAYER_MOVE" => return (None, "No equivalent NX layer-management operation is available"),
        "UG_WCS_ORIENT" => return (None, "NX working coordinate system orientation has no equivalent shortcut command"),
        "UG_FILE_EXPORT_PDF" => return (None, "No equivalent PDF export command is available"),
        "UG_INFO_DISPLAYED_PART" => return (None, "No equivalent displayed-part information command is available"),
        "UG_DRAFT_DRW_UPDATE_VIEW" => return (None, "No equivalent drawing-view update command is available"),
        "UG_MODELING_EDIT_FEATURE_DENSITY" => return (None, "NX feature density has no equivalent shortcut command"),
        "UG_MODELING_EDIT_FEATURE_REMOVE_PARAMETERS" => return (None, "No equivalent remove-feature-parameters operation is available"),
        "UG_FILE_CLOSE_SELECTED" => return (None, "Close selected NX parts differs from closing the active SolidCraft document"),
        "UG_MODELING_FF_SURF4P" => return (None, "No equivalent four-point surface command is available; Ctrl+Shift+S is also reserved for Save As"),
        _ => return (None, "No verified SolidCraft command mapping; choose a command in the shortcut workbench"),
    };
    (Some(id), "Mapped to the corresponding SolidCraft tool")
}

/// Keep existing alternate keys. The normal keymap binder owns canonicalization,
/// reserved keys, the eight-key limit, and cross-context collision policy.
pub(crate) fn apply(app: &mut SolveApp, report: &mut Report, replace_conflicts: bool) {
    let mut by_key = std::collections::BTreeMap::<String, Vec<&str>>::new();
    for entry in &report.entries {
        if let (Some(id), Some(key)) = (entry.command_id.as_deref(), keymap::canonical_key(&entry.key)) {
            let ids = by_key.entry(key).or_default();
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    let ambiguous: Vec<bool> = report
        .entries
        .iter()
        .map(|entry| {
            let Some(id) = entry.command_id.as_deref() else {
                return false;
            };
            let Some(key) = keymap::canonical_key(&entry.key) else {
                return false;
            };
            by_key.get(&key).into_iter().flatten().any(|&other_id| {
                let a = keymap::scope(id);
                let b = keymap::scope(other_id);
                id != other_id && (a == b || a == keymap::Scope::Everywhere || b == keymap::Scope::Everywhere)
            })
        })
        .collect();
    drop(by_key);
    for (entry, ambiguous) in report.entries.iter_mut().zip(ambiguous) {
        if matches!(entry.status, Status::Suppressed | Status::Unsupported) {
            continue;
        }
        let Some(id) = entry.command_id.as_deref() else {
            continue;
        };
        if ambiguous {
            entry.status = Status::Conflict;
            entry.note = "Multiple imported actions use this key in the same SolidCraft context; choose one in the workbench".into();
            continue;
        }
        let existing = keymap::keys(app, id);
        let canonical = keymap::canonical_key(&entry.key);
        if existing.iter().any(|key| Some(key) == canonical.as_ref()) {
            entry.status = Status::Applied;
            continue;
        }
        match keymap::bind_at(app, id, Some(existing.len()), &entry.key, replace_conflicts) {
            Ok(()) => entry.status = Status::Applied,
            Err(error) => {
                entry.status = Status::Conflict;
                entry.note = error;
            }
        }
    }
}
