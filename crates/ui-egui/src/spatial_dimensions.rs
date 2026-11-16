//! Driving spatial dimensions share the engine solver; this panel only collects operands.
use crate::{
    SolveApp, ui_kit,
    viewport::{Hit, Proj},
};
use egui::{Pos2, Stroke};
use serde_json::{Value, json};
use solidcore_record::Vec3;
use solidcraft_engine::{Sel, geom::prelude::*};
use solidcraft_ui_text::tr;

const KINDS: &[(&str, &str)] = &[
    ("distance", "ui.dim3.length"),
    ("x", "ui.dim3.x"),
    ("y", "ui.dim3.y"),
    ("z", "ui.dim3.z"),
    ("radius", "ui.dim3.radius"),
    ("angle", "ui.dim3.angle"),
    ("fix", "ui.dim3.fix"),
];
pub(crate) fn kind_label(kind: &str) -> String {
    let kind = match kind {
        "delta_x" => "x",
        "delta_y" => "y",
        "delta_z" => "z",
        other => other,
    };
    KINDS.iter().find(|k| k.0 == kind).map(|k| tr(k.1).to_string()).unwrap_or_else(|| kind.to_owned())
}
pub(crate) struct State {
    session: u64,
    sketch: u64,
    kind: usize,
    a: String,
    b: String,
    value: String,
    edit: Option<String>,
    pick_b: bool,
    error: Option<String>,
}

pub(crate) fn open(app: &mut SolveApp) {
    let Some(sketch) = app.session.active_sketch else { return };
    let picks: Vec<_> = app
        .session
        .selection
        .iter()
        .filter_map(|s| match s {
            Sel::SketchPoint { id } | Sel::SketchCurve { id, .. } if id.starts_with('w') => Some(id.clone()),
            _ => None,
        })
        .take(2)
        .collect();
    let mut state = State {
        session: app.session.uid,
        sketch,
        kind: 0,
        a: picks.first().cloned().unwrap_or_default(),
        b: picks.get(1).cloned().unwrap_or_default(),
        value: String::new(),
        edit: None,
        pick_b: false,
        error: None,
    };
    state.pick_b = !state.a.is_empty();
    measure(app, &mut state);
    app.spatial_dimensions = Some(state);
    app.tool = crate::tools::Tool::for_command("sketch.dimension");
}

fn operands(app: &SolveApp, state: &State) -> Option<(Vec3, Vec3)> {
    let world = app.session.model.state();
    let sk = &world.sketch(state.sketch)?.sketch;
    let point = |name: &str, last: bool| {
        let choices = sk.wire_operand(name)?;
        let p = if last { choices.last()? } else { choices.first()? };
        sk.wires.iter().find(|w| w.id == p.wire)?.fit.get(p.index).copied()
    };
    Some((point(&state.a, false)?, point(if state.b.is_empty() { &state.a } else { &state.b }, state.b.is_empty())?))
}
fn measure(app: &SolveApp, state: &mut State) {
    let world = app.session.model.state();
    let Some(ss) = world.sketch(state.sketch) else { return };
    let wire = |name: &str| ss.sketch.wires.iter().find(|w| w.id == name.split([':', '.']).next().unwrap_or(name));
    let kind = KINDS.get(state.kind).map_or("distance", |k| k.0);
    let value = match kind {
        "radius" => wire(&state.a).and_then(|w| w.arc.map(|a| a.radius)),
        "angle" => wire(&state.a).zip(wire(&state.b)).and_then(|(a, b)| {
            let u = *a.fit.last()? - *a.fit.first()?;
            let v = *b.fit.last()? - *b.fit.first()?;
            let den = u.len() * v.len();
            (den > 1e-12).then(|| (u.dot(v) / den).clamp(-1., 1.).acos().to_degrees())
        }),
        "fix" => None,
        _ => operands(app, state).map(|(a, b)| match kind {
            "x" => b.x - a.x,
            "y" => b.y - a.y,
            "z" => b.z - a.z,
            _ => a.dist(b),
        }),
    };
    state.value = value.map(|v| format!("{v:.6}")).unwrap_or_default();
}

pub(crate) fn on_click(app: &mut SolveApp, proj: &Proj, pos: Pos2) -> bool {
    if app.spatial_dimensions.is_none() || !app.tool.as_ref().is_some_and(|t| t.cmd == "sketch.dimension") {
        return false;
    }
    let Some(mut state) = app.spatial_dimensions.take() else { return false };
    let id = crate::viewport::pick_3d(app, proj, pos).iter().find_map(|h| match h {
        Hit::SketchPoint3d { sketch, id, .. } | Hit::SketchCurve { sketch, id, .. } if *sketch == state.sketch && id.starts_with('w') => {
            Some(id.clone())
        }
        _ => None,
    });
    if let Some(id) = id {
        if state.pick_b && !matches!(state.kind, 4 | 6) {
            state.b = id;
        } else {
            state.a = id;
            state.b.clear();
            state.pick_b = true;
        }
        state.edit = None;
        state.error = None;
        if state.b.is_empty() && state.kind == 0 {
            let world = app.session.model.state();
            if world.sketch(state.sketch).is_some_and(|s| s.sketch.wires.iter().any(|w| w.id == state.a && w.arc.is_some())) {
                state.kind = 4;
            }
        }
        measure(app, &mut state);
    }
    app.spatial_dimensions = Some(state);
    true
}
fn button(ui: &mut egui::Ui, id: &str, label: &str) -> bool {
    let r = ui.button(tr(label));
    solidcraft_ui_registry::publish_handle(id, r.rect.center());
    r.clicked()
}

pub(crate) fn edit(app: &mut SolveApp, id: &str) {
    open(app);
    let Some(mut state) = app.spatial_dimensions.take() else { return };
    let world = app.session.model.state();
    if let Some(c) = world.sketch(state.sketch).and_then(|s| s.sketch.wire_constraints.iter().find(|c| c.id == id)) {
        let v = serde_json::to_value(c).unwrap_or(Value::Null);
        load_edit(&mut state, &v);
    }
    app.spatial_dimensions = Some(state);
}
fn load_edit(state: &mut State, v: &Value) {
    let kind = v["kind"].as_str().unwrap_or("");
    let kind = match kind {
        "delta_x" => "x",
        "delta_y" => "y",
        "delta_z" => "z",
        other => other,
    };
    state.kind = KINDS.iter().position(|k| k.0 == kind).unwrap_or(0);
    state.edit = v["id"].as_str().map(str::to_owned);
    let operand = |p: &Value| format!("{}:{}", p["wire"].as_str().unwrap_or(""), p["index"].as_u64().unwrap_or(0));
    state.a = operand(&v["a"]);
    state.b = operand(&v["b"]);
    state.value = v["value"].as_f64().map(|x| format!("{:.6}", if kind == "angle" { x.to_degrees() } else { x })).unwrap_or_default();
    state.error = None;
}

pub(crate) fn show(app: &mut SolveApp, ctx: &egui::Context) {
    let Some(mut state) = app.spatial_dimensions.take() else { return };
    if app.session.uid != state.session || app.session.active_sketch != Some(state.sketch) {
        return;
    }
    let mut apply = false;
    let mut remeasure = false;
    let mut remove = None;
    let mut edit_value = None;
    let world = app.session.model.state();
    let relations: Vec<Value> = world
        .sketch(state.sketch)
        .map(|s| s.sketch.wire_constraints.iter().filter_map(|c| serde_json::to_value(c).ok()).collect())
        .unwrap_or_default();
    let (closed, action) = ui_kit::Window::new(egui::Id::unique("spatial_dimensions"), "dimension", tr("ui.dim3.title"))
        .fields()
        .anchor(egui::Align2::LEFT_TOP, egui::vec2(280., 190.))
        .actions("", tr("ui.close"), None)
        .show(ctx, true, |ui| {
            ui.add(egui::Label::new(tr("ui.dim3.help")).wrap());
            ui_kit::row(ui, tr("ui.dim3.kind"), |ui| {
                let labels: Vec<_> = KINDS.iter().map(|k| tr(k.1)).collect();
                let old = state.kind;
                ui_kit::dropdown(ui, "spatial_dimension_kind", &mut state.kind, &labels.iter().map(|s| s.as_ref()).collect::<Vec<&str>>());
                remeasure |= old != state.kind;
            });
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("A: {}", state.a));
                if button(ui, "dim3:pick_a", "ui.dim3.pick_a") {
                    state.pick_b = false;
                    app.tool = crate::tools::Tool::for_command("sketch.dimension");
                }
            });
            if !matches!(state.kind, 4 | 6) {
                ui.horizontal_wrapped(|ui| {
                    ui.label(format!("B: {}", state.b));
                    if button(ui, "dim3:pick_b", "ui.dim3.pick_b") {
                        state.pick_b = true;
                        app.tool = crate::tools::Tool::for_command("sketch.dimension");
                    }
                });
            }
            if matches!(state.kind, 1..=3) {
                ui.add(egui::Label::new(tr("ui.dim3.axes_note")).wrap());
            }
            if state.kind != 6 {
                ui_kit::row(ui, tr(if state.kind == 5 { "ui.dim3.degrees" } else { "ui.dim3.mm" }), |ui| {
                    let r = ui.text_edit_singleline(&mut state.value);
                    solidcraft_ui_registry::publish_handle("dim3:value", r.rect.center());
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        apply = true;
                    }
                });
            }
            ui.horizontal(|ui| {
                if button(ui, "dim3:apply", if state.edit.is_some() { "ui.dim3.update" } else { "ui.dim3.apply" }) {
                    apply = true;
                }
                if button(ui, "dim3:new", "ui.dim3.new") {
                    state.edit = None;
                    state.a.clear();
                    state.b.clear();
                    state.value.clear();
                    state.pick_b = false;
                    state.error = None;
                    app.tool = crate::tools::Tool::for_command("sketch.dimension");
                }
            });
            if let Some(error) = &state.error {
                ui_kit::text(ui, error, ui_kit::Text::Error("error"));
            }
            ui.separator();
            ui.label(tr("ui.dim3.saved"));
            egui::ScrollArea::vertical().max_height(220.).show(ui, |ui| {
                for v in &relations {
                    let Some(id) = v["id"].as_str() else { continue };
                    ui.horizontal_wrapped(|ui| {
                        let kind = v["kind"].as_str().unwrap_or("");
                        let value = v["value"]
                            .as_f64()
                            .map(|x| format!("{:.3} {}", if kind == "angle" { x.to_degrees() } else { x }, if kind == "angle" { "°" } else { "mm" }))
                            .unwrap_or_default();
                        ui.label(format!("{id} · {} {value}", kind_label(kind)));
                        if v["value"].is_number() && button(ui, &format!("dim3:edit:{id}"), "ui.dim3.edit") {
                            edit_value = Some(v.clone());
                        }
                        if button(ui, &format!("dim3:remove:{id}"), "ui.dim3.remove") {
                            remove = Some(id.to_owned());
                        }
                    });
                }
            });
            if button(ui, "dim3:report", "ui.dim3.report") {
                crate::sketch_report::open(app);
            }
        });
    if closed || action != ui_kit::Action::None {
        if app.tool.as_ref().is_some_and(|t| t.cmd == "sketch.dimension") {
            app.tool = None;
        }
        return;
    }
    if remeasure {
        measure(app, &mut state);
    }
    if let Some(v) = edit_value {
        load_edit(&mut state, &v);
    }
    if let Some(id) = remove {
        state.error = app.run("sketch.delete", json!({"sketch":state.sketch,"entities":[id]})).err();
    }
    if apply {
        let kind = KINDS.get(state.kind).map_or("distance", |k| k.0);
        let value = state.value.parse::<f64>().ok().filter(|v| v.is_finite());
        if kind != "fix" && value.is_none() {
            state.error = Some(tr("ui.dim3.number").to_string());
        } else {
            let (command, mut params) = if let Some(id) = &state.edit {
                ("sketch.dimension3d.edit", json!({"sketch":state.sketch,"id":id,"value":value}))
            } else {
                ("sketch.dimension3d", json!({"sketch":state.sketch,"kind":kind,"a":state.a}))
            };
            if state.edit.is_none() {
                if !state.b.is_empty() && !matches!(kind, "radius" | "fix") {
                    params["b"] = state.b.clone().into();
                }
                if let Some(value) = value {
                    params["value"] = value.into();
                }
            }
            match app.run(command, params) {
                Ok(result) => {
                    state.error = None;
                    state.edit = if kind == "fix" { None } else { result["constraints"][0].as_str().map(str::to_owned) };
                }
                Err(error) => state.error = Some(error),
            }
        }
    }
    app.spatial_dimensions = Some(state);
}

pub(crate) fn paint(app: &SolveApp, painter: &egui::Painter, proj: &Proj) {
    let world = crate::open_meshing::session_world(&app.session);
    let Some(ss) = app.session.active_sketch.and_then(|id| world.sketch(id)) else { return };
    if ss.sketch.view.hide_dimensions {
        return;
    }
    let tk = solidcraft_theme::Tokens::view();
    for (i, c) in ss.sketch.wire_constraints.iter().enumerate() {
        let v = serde_json::to_value(c).unwrap_or(Value::Null);
        let Some(value) = v["value"].as_f64() else { continue };
        let point = |p: &Value| {
            ss.sketch
                .wires
                .iter()
                .find(|w| Some(w.id.as_str()) == p["wire"].as_str())
                .and_then(|w| w.fit.get(usize::try_from(p["index"].as_u64()?).ok()?))
                .copied()
        };
        let Some((mut a, b)) = point(&v["a"]).zip(point(&v["b"])) else { continue };
        if v["kind"] == "radius" {
            if let Some(center) = ss.sketch.wires.iter().find(|w| w.id == c.a.wire).and_then(|w| w.arc.map(|arc| arc.center)) {
                a = center;
            }
        }
        let Some((a, b)) = proj.to_screen(a).zip(proj.to_screen(b)) else { continue };
        let kind = v["kind"].as_str().unwrap_or("");
        let offset = egui::vec2(18., -24. - (i % 4) as f32 * 20.);
        if a.distance(b) > 24. {
            let along = (b - a).normalized() * 10.;
            painter.arrow(a + offset + along, -along, Stroke::new(1., tk.accent));
            painter.arrow(b + offset - along, along, Stroke::new(1., tk.accent));
        }
        let at = a + (b - a) * 0.5 + offset;
        painter.line_segment([a, a + offset], Stroke::new(1., tk.accent));
        painter.line_segment([b, b + offset], Stroke::new(1., tk.accent));
        painter.line_segment([a + offset, b + offset], Stroke::new(1., tk.accent));
        let label = format!(
            "{} · {} {:.3}{}",
            c.id,
            kind_label(kind),
            if kind == "angle" { value.to_degrees() } else { value },
            if kind == "angle" { "°" } else { " mm" }
        );
        painter.text(
            at,
            egui::Align2::CENTER_BOTTOM,
            label,
            egui::TextStyle::Body.resolve(&painter.ctx().style_of(painter.ctx().theme())),
            tk.accent,
        );
    }
}
