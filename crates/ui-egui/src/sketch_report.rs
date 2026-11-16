//! The Sketch report window (#6262): what `sketch.inspect` says about the active sketch, for
//! people. The solver status and degrees of freedom left, how many curves, points and
//! constraints the sketch has, and which curves and points are not fully constrained yet.
//! Copy JSON puts the whole `sketch.inspect` result on the clipboard. `ui.start` of
//! `sketch.inspect` (its ribbon button) opens it; `engine.execute` still only returns the JSON.
//! The report follows edits while it is open.

use serde_json::Value;

use crate::SolveApp;
use crate::ui_kit;

/// Rows of unconstrained curves and points shown before the list scrolls.
const LIST_ROWS: f32 = 8.0;

/// The open report: the last `sketch.inspect` result, or why there is none.
pub struct SketchReport {
    /// The session revision the report was read at.
    pub revision: u64,
    /// The sketch the report is about (the active one when the window opened).
    pub sketch: Option<u64>,
    pub result: Result<Value, String>,
}

fn read(app: &mut SolveApp, sketch: Option<u64>) -> Result<Value, String> {
    let params = match sketch {
        Some(id) => serde_json::json!({ "sketch": id }),
        None => serde_json::json!({}),
    };
    app.session.execute("sketch.inspect", &params).map_err(|e| e.to_string())
}

/// Open the report on the active sketch.
pub fn open(app: &mut SolveApp) {
    let sketch = app.session.active_sketch;
    let result = read(app, sketch);
    app.sketch_report = Some(SketchReport { revision: app.session.revision, sketch, result });
}

/// The ids of the entries in `list` that are not fully constrained.
pub fn loose(v: &Value, list: &str) -> Vec<String> {
    v[list]
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|e| !e["fully_constrained"].as_bool().unwrap_or(false))
                .map(|e| match e["type"].as_str() {
                    Some(kind) => format!("{} ({kind})", e["id"].as_str().map(str::to_string).unwrap_or_else(|| e["id"].to_string())),
                    None => e["id"].as_str().map(str::to_string).unwrap_or_else(|| e["id"].to_string()),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn count(v: &Value, list: &str) -> usize {
    v[list].as_array().map_or(0, Vec::len)
}

/// The solver status as a word ("well", "under", …).
fn status(v: &Value) -> String {
    match &v["status"] {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Draw the window when it is open. Called once per frame.
pub fn show(app: &mut SolveApp, ctx: &egui::Context) {
    let Some(mut r) = app.sketch_report.take() else { return };
    if r.revision != app.session.revision {
        r.revision = app.session.revision;
        r.result = read(app, r.sketch);
    }
    let mut remove_spatial = None;
    let mut edit_spatial = None;
    let title = solidcraft_ui_text::tr_label("Sketch Report");
    let (closed, action) = ui_kit::Window::new(egui::Id::unique("sc_sketch_report"), "inspect", title)
        .fields()
        .actions("", solidcraft_ui_text::tr("ui.close"), None)
        .show(ctx, true, |ui| match &r.result {
            Err(e) => {
                ui_kit::text(ui, e, ui_kit::Text::Error("error"));
            }
            Ok(v) => {
                let name = v["name"].as_str().unwrap_or_default();
                let spatial = count(v, "wires") > 0;
                ui_kit::row(ui, solidcraft_ui_text::tr_label("Sketch"), |ui| ui_kit::text(ui, name, ui_kit::Text::Cell));
                ui_kit::row(ui, solidcraft_ui_text::tr_label(if spatial { "Planar status" } else { "Status" }), |ui| {
                    ui_kit::text(ui, &status(v), ui_kit::Text::Cell)
                });
                let dof = v["dof"].as_u64().map(|d| d.to_string()).unwrap_or_else(|| v["dof"].to_string());
                ui_kit::row(ui, solidcraft_ui_text::tr_label(if spatial { "Planar degrees of freedom" } else { "Degrees of freedom" }), |ui| {
                    ui_kit::text(ui, &dof, ui_kit::Text::Cell)
                });
                let construction = v["curves"].as_array().map_or(0, |a| a.iter().filter(|c| c["construction"].as_bool() == Some(true)).count());
                let curves =
                    if construction > 0 { format!("{} ({construction} construction)", count(v, "curves")) } else { count(v, "curves").to_string() };
                ui_kit::row(ui, solidcraft_ui_text::tr_label("Curves"), |ui| ui_kit::text(ui, &curves, ui_kit::Text::Cell));
                for (label, list) in [("Points", "points"), ("Constraints", "constraints"), ("Profiles", "profiles")] {
                    ui_kit::row(ui, solidcraft_ui_text::tr_label(label), |ui| ui_kit::text(ui, &count(v, list).to_string(), ui_kit::Text::Cell));
                }
                if spatial {
                    ui_kit::row(ui, solidcraft_ui_text::tr_label("3D constraints"), |ui| {
                        ui_kit::text(ui, &count(v, "wire_constraints").to_string(), ui_kit::Text::Cell)
                    });
                    let freedom = &v["spatial_freedom"];
                    let dof =
                        v["spatial_dof"].as_u64().map(|n| n.to_string()).unwrap_or_else(|| solidcraft_ui_text::tr("ui.dim3.unknown").to_string());
                    ui_kit::row(ui, solidcraft_ui_text::tr("ui.dim3.dof"), |ui| ui.label(dof));
                    ui.add(egui::Label::new(solidcraft_ui_text::tr("ui.dim3.freedom_note")).wrap());
                    if let Some(reason) = freedom["reason"].as_str() {
                        ui.add(egui::Label::new(reason).wrap());
                    }
                    if let Some(points) = freedom["points"].as_array() {
                        egui::ScrollArea::vertical().id_salt("spatial_free_points").max_height(180.).show(ui, |ui| {
                            for p in points {
                                let axes: Vec<_> = ["X", "Y", "Z"]
                                    .iter()
                                    .enumerate()
                                    .filter_map(|(i, axis)| (p["free_axes"][i].as_bool() == Some(true)).then_some(*axis))
                                    .collect();
                                let motion = if axes.is_empty() { solidcraft_ui_text::tr("ui.dim3.fixed").to_string() } else { axes.join(" / ") };
                                ui.label(format!("{}:{} · {}", p["wire"].as_str().unwrap_or(""), p["index"], motion));
                            }
                        });
                    }
                    if let Some(relations) = v["wire_constraints"].as_array() {
                        for relation in relations {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(format!(
                                    "{} · {} · {}:{} / {}:{}",
                                    relation["id"].as_str().unwrap_or(""),
                                    crate::spatial_dimensions::kind_label(relation["kind"].as_str().unwrap_or("")),
                                    relation["a"]["wire"].as_str().unwrap_or(""),
                                    relation["a"]["index"],
                                    relation["b"]["wire"].as_str().unwrap_or(""),
                                    relation["b"]["index"]
                                ));
                                if let Some(id) = relation["id"].as_str() {
                                    if let Some(value) = relation["value"].as_f64() {
                                        let angle = relation["kind"] == "angle";
                                        ui.label(format!("{:.3} {}", if angle { value.to_degrees() } else { value }, if angle { "°" } else { "mm" }));
                                        let edit = ui.button(solidcraft_ui_text::tr("ui.dim3.edit"));
                                        solidcraft_ui_registry::publish_handle(&format!("spatial_constraint:edit:{id}"), edit.rect.center());
                                        if edit.clicked() {
                                            edit_spatial = Some(id.to_owned());
                                        }
                                    }
                                    let response = ui.button(solidcraft_ui_text::tr_label("Remove"));
                                    solidcraft_ui_registry::publish_handle(&format!("spatial_constraint:remove:{id}"), response.rect.center());
                                    if response.clicked() {
                                        remove_spatial = Some(id.to_owned());
                                    }
                                }
                            });
                        }
                    }
                }
                let (curves, points) = (loose(v, "curves"), loose(v, "points"));
                if curves.is_empty() && points.is_empty() && !spatial {
                    ui_kit::text(ui, solidcraft_ui_text::tr_label("Everything is fully constrained."), ui_kit::Text::Note("all_constrained"));
                } else if !curves.is_empty() || !points.is_empty() {
                    let heading = format!("{} ({})", solidcraft_ui_text::tr_label("Not fully constrained"), curves.len() + points.len());
                    ui_kit::section(ui, "sc_sketch_report_loose", &heading, true, |ui| {
                        egui::ScrollArea::vertical().max_height(ui_kit::row_height() * LIST_ROWS).show(ui, |ui| {
                            for (label, ids) in [("Curve", &curves), ("Point", &points)] {
                                for id in ids {
                                    ui_kit::row(ui, solidcraft_ui_text::tr_label(label), |ui| ui_kit::text(ui, id, ui_kit::Text::Cell));
                                }
                            }
                        });
                    });
                }
                if ui_kit::icon_label_button(ui, solidcraft_ui_text::tr_label("Copy JSON"), "copy", true, false).clicked() {
                    ctx.copy_text(serde_json::to_string_pretty(v).unwrap_or_default());
                }
            }
        });
    if closed || action != ui_kit::Action::None {
        return;
    }
    if let Some(id) = edit_spatial {
        crate::spatial_dimensions::edit(app, &id);
    }
    if let Some(id) = remove_spatial {
        let mut params = serde_json::json!({"entities":[id]});
        if let Some(sketch) = r.sketch {
            params["sketch"] = sketch.into();
        }
        if let Err(error) = app.run("sketch.delete", params) {
            r.result = Err(error);
        }
    }
    app.sketch_report = Some(r);
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::scenario::Harness;

    #[test]
    fn the_ribbon_button_opens_a_report_that_follows_the_sketch() {
        let mut h = Harness::new();
        h.app.home.open = false;
        h.app.run("sketch.create", json!({"plane": "XY"})).unwrap();
        h.app.run("sketch.line", json!({"points": [[0, 0], [10, 0]]})).unwrap();
        h.step(&json!({"start": "sketch.inspect"})).unwrap();
        h.frames(2);
        let r = h.app.sketch_report.as_ref().expect("the button opens the report");
        let v = r.result.as_ref().expect("the active sketch is inspected");
        assert_eq!(v["curves"].as_array().map(Vec::len), Some(1));
        assert_eq!(super::loose(v, "curves").len(), 1, "a free line is not fully constrained");
        h.app.run("sketch.line", json!({"points": [[10, 0], [10, 10]]})).unwrap();
        h.frames(2);
        let v = h.app.sketch_report.as_ref().unwrap().result.as_ref().unwrap();
        assert_eq!(v["curves"].as_array().map(Vec::len), Some(2), "the report follows edits");
        // Closing it changes nothing in the design.
        let undo = h.app.session.undo.len();
        h.frames(4);
        assert!(crate::scenario::handle_at("dialog:cancel").is_some(), "the window is drawn");
        h.step(&json!({"click": {"handle": "dialog:cancel"}})).unwrap();
        h.frames(2);
        assert!(h.app.sketch_report.is_none());
        assert_eq!(h.app.session.undo.len(), undo);
    }

    #[test]
    fn the_command_is_on_the_sketch_tab() {
        let spec = solidcraft_engine::find_command("sketch.inspect").unwrap();
        assert_eq!((spec.tab, spec.panel), ("SKETCH", "INSPECT"));
        // While a sketch is open the Sketch tab shows it, not the Inspect tab's drop-down.
        let mut h = Harness::new();
        h.app.home.open = false;
        h.app.run("sketch.create", json!({"plane": "XY"})).unwrap();
        h.frames(3);
        let at = crate::scenario::handle_at("toolbar:sketch.inspect").expect("the button is on the ribbon");
        h.step(&json!({"click": [at.x, at.y]})).unwrap();
        h.frames(2);
        assert!(h.app.sketch_report.is_some(), "clicking the ribbon button opens the report");
    }
    #[test]
    fn spatial_dimension_panel_drives_geometry_and_edits_saved_value() {
        let mut h = Harness::new();
        let id = h.app.run("sketch.create", json!({"plane":"XY"})).unwrap()["sketch"].as_u64().unwrap();
        h.app.run("sketch.options", json!({"sketch_3d":true})).unwrap();
        h.app.run("sketch.line3d", json!({"points":[[0,0,10],[10,10,20]]})).unwrap();
        h.app.session.selection = vec![solidcraft_engine::Sel::SketchCurve { id: "w1".into(), sketch: None }];
        h.app.start("sketch.dimension");
        h.frames(3);
        assert!(h.app.spatial_dimensions.is_some());
        h.step(&json!({"click":{"handle":"dim3:value"}})).unwrap();
        h.call("ui.key", json!({"key":"A","ctrl":true}));
        h.call("ui.text", json!({"text":"25"}));
        h.frames(2);
        h.step(&json!({"click":{"handle":"dim3:apply"}})).unwrap();
        h.frames(3);
        let report = h.app.run("sketch.inspect", json!({})).unwrap();
        assert_eq!(report["spatial_dof"], 5);
        let c = report["wire_constraints"][0]["id"].as_str().unwrap().to_owned();
        assert_eq!(report["wire_constraints"][0]["value"], 25.);
        h.step(&json!({"click":{"handle":format!("dim3:edit:{c}")}})).unwrap();
        h.frames(2);
        h.step(&json!({"click":{"handle":"dim3:value"}})).unwrap();
        h.call("ui.key", json!({"key":"A","ctrl":true}));
        h.call("ui.text", json!({"text":"30"}));
        h.frames(2);
        h.step(&json!({"click":{"handle":"dim3:apply"}})).unwrap();
        h.frames(3);
        let sk = h.app.session.doc.sketch(id).unwrap();
        use solidcraft_engine::geom::prelude::*;
        assert!((sk.wires[0].fit[0].dist(sk.wires[0].fit[1]) - 30.).abs() < 1e-6);
        assert_eq!(sk.wire_constraints.len(), 1, "edit updates existing dimension");
    }
}
