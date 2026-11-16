//! Geometry picking for movable coordinate frames; no document mutations occur here.
use crate::{
    SolveApp, gizmo, move_handles, ui_kit,
    viewport::{Hit, Proj},
};
use egui::{Context, Id, Pos2, Stroke};
use solidcore_record::Vec3;
use solidcraft_engine::{
    doc::Mat,
    geometry_snap::{GeometryKind, GeometryPick, GeometrySnap, SnapMode},
};

#[derive(Clone)]
struct Inferred {
    snap: GeometrySnap,
    guides: Vec<[f64; 3]>,
}
#[derive(Clone)]
struct State {
    owner: String,
    frame: Mat,
    request: Option<Option<usize>>,
    mode: usize,
    result: Option<Mat>,
    first: Option<(GeometryPick, Option<Mat>)>,
    marker: Option<GeometrySnap>,
    first_marker: Option<GeometrySnap>,
    second_marker: Option<GeometrySnap>,
    error: Option<String>,
    last_frame: u64,
    cache: std::collections::HashMap<String, GeometrySnap>,
    revision: u64,
    dragging_origin: bool,
    selected_axis: usize,
    inferred: Vec<Inferred>,
    hovered_lines: Vec<(GeometryPick, Option<Mat>, [f64; 3])>,
    last_line: Option<String>,
    preview_chain: Option<(Vec<String>, Mat)>,
    hovering_marker: bool,
    last_axis_click: Option<usize>,
}
fn key() -> Id {
    Id::unique("move-geometry-snap")
}
fn state(ctx: &Context) -> Option<State> {
    ctx.data(|d| d.get_temp(key()))
}
fn put(ctx: &Context, s: State) {
    ctx.data_mut(|d| d.insert_temp(key(), s));
}
pub fn active_owner(ctx: &Context) -> Option<String> {
    state(ctx).filter(|s| s.request.is_some() || s.result.is_some() || s.hovering_marker).map(|s| s.owner)
}
pub fn active(ctx: &Context) -> bool {
    active_owner(ctx).is_some()
}
pub(crate) fn marker_owns_pointer(ctx: &Context) -> bool {
    state(ctx).is_some_and(|s| s.hovering_marker || s.result.is_some())
}
/// A deliberate triad drag replaces an armed geometry pick (click and drag have different jobs).
pub(crate) fn cancel_for_handle_drag(ctx: &Context) {
    if let Some(mut s) = state(ctx) {
        s.request = None;
        s.hovering_marker = false;
        s.marker = None;
        s.first = None;
        s.dragging_origin = false;
        put(ctx, s);
    }
}
/// A centre drag picks a geometric target on release instead of moving in a camera plane.
pub fn begin_drag(ctx: &Context) -> bool {
    let Some(mut s) = state(ctx) else { return false };
    s.request = Some(None);
    s.dragging_origin = true;
    s.first = None;
    s.error = None;
    put(ctx, s);
    true
}
/// Arm the same geometry picker used by the popup from a triad handle.
pub fn handle_click(ctx: &Context, h: gizmo::Handle) {
    if matches!(h, gizmo::Handle::Ring(_) | gizmo::Handle::Plane(_)) {
        cancel_for_handle_drag(ctx);
        return;
    }
    let Some(mut s) = state(ctx) else { return };
    if let gizmo::Handle::Axis(axis) = h
        && s.last_axis_click == Some(axis)
        && ctx.input(|i| i.pointer.button_double_clicked(egui::PointerButton::Primary))
    {
        if let Some(direction) = move_handles::axes(&s.frame).get(axis) {
            if let Ok(next) = solidcraft_engine::geometry_snap::align_axis(&s.frame, axis, [direction.x, direction.y, direction.z], true) {
                s.selected_axis = axis;
                s.result = Some(next);
                s.request = None;
                s.error = None;
                s.last_axis_click = None;
                put(ctx, s);
            }
        }
        return;
    }
    s.request = match h {
        gizmo::Handle::Ball => Some(None),
        gizmo::Handle::Axis(i) => Some(Some(i)),
        _ => return,
    };
    s.last_axis_click = match h {
        gizmo::Handle::Axis(axis) => Some(axis),
        _ => None,
    };
    if let Some(Some(axis)) = s.request {
        s.selected_axis = axis;
    }
    s.first = None;
    s.marker = None;
    s.error = None;
    s.dragging_origin = false;
    put(ctx, s);
}
/// Commit one exact engine candidate to the active draft frame.
pub fn accept(ctx: &Context, snap: &GeometrySnap) -> bool {
    if snap.point.iter().any(|v| !v.is_finite() || v.abs() > 1e9) {
        return false;
    }
    let Some(mut s) = state(ctx) else { return false };
    let Some(axis) = s.request else { return false };
    let next = if let Some(axis) = axis {
        let Some(direction) = snap.direction else { return false };
        solidcraft_engine::geometry_snap::align_axis(&s.frame, axis, direction, false).ok()
    } else {
        let mut frame = s.frame;
        frame[3] = [snap.point[0], snap.point[1], snap.point[2], 1.];
        Some(frame)
    };
    let Some(next) = next else { return false };
    s.result = Some(next);
    s.request = None;
    s.dragging_origin = false;
    s.first = None;
    s.marker = Some(snap.clone());
    put(ctx, s);
    true
}
/// Call for both handles-only and part-moving modes; the caller applies the returned frame.
pub fn rows(_app: &SolveApp, ui: &mut egui::Ui, owner: &str, frame: &mut Mat) -> bool {
    let mut s =
        state(ui.ctx()).filter(|s| s.owner == owner && ui.ctx().cumulative_frame_nr().saturating_sub(s.last_frame) < 3).unwrap_or_else(|| State {
            owner: owner.into(),
            frame: *frame,
            request: None,
            mode: 0,
            result: None,
            first: None,
            marker: None,
            first_marker: None,
            second_marker: None,
            error: None,
            last_frame: 0,
            cache: Default::default(),
            revision: 0,
            dragging_origin: false,
            selected_axis: 2,
            inferred: Vec::new(),
            hovered_lines: Vec::new(),
            last_line: None,
            preview_chain: None,
            hovering_marker: false,
            last_axis_click: None,
        });
    s.last_frame = ui.ctx().cumulative_frame_nr();
    let changed = s.result.take().is_some_and(|next| {
        *frame = next;
        true
    });
    s.frame = *frame;
    let modes = ["move.snap.auto", "move.snap.point", "move.snap.midpoint", "move.snap.center", "move.snap.intersection"].map(solidcraft_ui_text::tr);
    let refs: Vec<&str> = modes.iter().map(|s| s.as_ref()).collect();
    let old_mode = s.mode;
    let r = ui_kit::choice_row(ui, solidcraft_ui_text::tr("move.snap.mode"), "move-snap-mode", &mut s.mode, &refs);
    crate::scenario::publish_handle("move-snap:mode", r.rect.center());
    if s.mode != old_mode {
        // Choosing a target type is itself an instruction to pick, including after a completed snap.
        s.request = Some(s.request.flatten());
        s.first = None;
        s.marker = None;
        s.error = None;
    }
    ui.horizontal_wrapped(|ui| {
        let r = ui.selectable_label(s.request == Some(None), solidcraft_ui_text::tr("move.snap.origin"));
        crate::scenario::publish_handle("move-snap:origin", r.rect.center());
        if r.clicked() {
            s.request = Some(None);
            s.first = None;
            s.error = None;
        }
        for (i, label) in ["move.snap.axis_x", "move.snap.axis_y", "move.snap.axis_z"].iter().enumerate() {
            let r = ui.selectable_label(s.request == Some(Some(i)), solidcraft_ui_text::tr(label));
            crate::scenario::publish_handle(&format!("move-snap:axis:{i}"), r.rect.center());
            if r.clicked() {
                s.request = Some(Some(i));
                s.selected_axis = i;
                s.first = None;
                s.error = None;
            }
        }
    });
    let mut flipped = false;
    ui.horizontal(|ui| {
        let r = ui.button(solidcraft_ui_text::tr("move.snap.flip"));
        crate::scenario::publish_handle("move-snap:flip", r.rect.center());
        if r.clicked() {
            let axis = s.selected_axis;
            let d = move_handles::axes(frame)[axis.min(2)];
            if let Ok(next) = solidcraft_engine::geometry_snap::align_axis(frame, axis, [d.x, d.y, d.z], true) {
                *frame = next;
                s.frame = next;
                flipped = true;
            }
        }
        if s.request.is_some() {
            let r = ui.button(solidcraft_ui_text::tr("move.snap.cancel"));
            crate::scenario::publish_handle("move-snap:cancel", r.rect.center());
            if r.clicked() {
                s.request = None;
                s.first = None;
            }
        }
    });
    if s.request.is_some() {
        ui_kit::text(
            ui,
            solidcraft_ui_text::tr(if s.request.flatten().is_some() {
                "move.snap.axis_prompt"
            } else if s.first.is_some() {
                "move.snap.second_edge"
            } else if s.mode == 4 {
                "move.snap.first_edge"
            } else if s.mode == 3 {
                "move.snap.center_prompt"
            } else {
                "move.snap.prompt"
            }),
            ui_kit::Text::Hint("geometry-snap"),
        );
    }
    if s.mode == 0 && _app.preferences.move_inference_markers {
        ui_kit::text(ui, solidcraft_ui_text::tr("move.snap.inference_hint"), ui_kit::Text::Hint("inferred-snaps"));
    }
    if s.request.is_none() {
        ui_kit::text(ui, solidcraft_ui_text::tr("move.snap.handle_gestures"), ui_kit::Text::Hint("handle-gestures"));
    }
    if let Some(error) = &s.error {
        ui_kit::text(ui, error, ui_kit::Text::Hint("snap-error"));
    }
    put(ui.ctx(), s);
    changed || flipped
}
fn pick(hit: &Hit) -> Option<GeometryPick> {
    let (body, kind, index, p) = match hit {
        Hit::Vertex { body, point } => (body, GeometryKind::Vertex, None, *point),
        Hit::Edge { body, index, point, .. } => (body, GeometryKind::Edge, Some(*index), *point),
        Hit::Face { body, index, point } => (body, GeometryKind::Face, Some(*index), *point),
        _ => return None,
    };
    Some(GeometryPick { body: body.clone(), kind, index, point: [p.x, p.y, p.z] })
}
/// Called before normal viewport selection. Returns true while picking owns primary input.
pub fn viewport(app: &SolveApp, ui: &egui::Ui, proj: &Proj, pos: Option<Pos2>, inside: bool, click: bool) -> bool {
    let Some(mut s) = state(ui.ctx()) else { return false };
    if ui.ctx().cumulative_frame_nr().saturating_sub(s.last_frame) > 2 {
        ui.ctx().data_mut(|d| d.remove::<State>(key()));
        return false;
    }
    let inference = s.mode == 0 && app.preferences.move_inference_markers;
    if !inference && (!s.inferred.is_empty() || s.hovering_marker) {
        s.hovering_marker = false;
        s.inferred.clear();
        s.hovered_lines.clear();
        s.last_line = None;
        put(ui.ctx(), s.clone());
    }
    if s.request.is_none() && !inference {
        return false;
    }
    let click = click || (s.dragging_origin && ui.input(|i| i.pointer.primary_released()));
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        s.request = None;
        s.first = None;
        put(ui.ctx(), s);
        return true;
    }
    if s.revision != app.session.revision {
        s.cache.clear();
        s.inferred.clear();
        s.marker = None;
        s.hovered_lines.clear();
        s.last_line = None;
        s.revision = app.session.revision;
    }
    let chain = crate::dialogs_move::chained(app);
    if s.preview_chain != chain {
        s.inferred.clear();
        s.hovered_lines.clear();
        s.last_line = None;
        s.preview_chain = chain.clone();
        s.marker = None;
    }
    let Some(pos) = pos.filter(|_| inside) else {
        let owns = s.request.is_some();
        s.hovering_marker = false;
        put(ui.ctx(), s);
        return owns;
    };
    // A triad handle wins over geometry and inferred markers underneath it. Keep the
    // pending pick while hovering the triad so another axis can be selected directly.
    let triad = app.dialog.as_ref().and_then(|d| crate::dialogs_move::triad(app, d)).or_else(|| crate::browser::occurrence_triad_model(app));
    if (s.request.is_some() || !inference) && triad.as_ref().is_some_and(|t| gizmo::contains_pointer(proj, t, pos)) {
        s.hovering_marker = false;
        s.marker = None;
        put(ui.ctx(), s);
        return false;
    }
    let preview = |p: &GeometryPick| chain.as_ref().filter(|(names, _)| names.contains(&p.body)).map(|(_, m)| *m);
    let unpreview = |p: &GeometryPick| {
        let transform = preview(p);
        let mut base = p.clone();
        if let Some(m) = transform.and_then(|m| solidcraft_engine::origins::frame_delta(&m, &solidcraft_engine::doc::IDENTITY).ok()) {
            let q = solidcraft_engine::doc::apply_point(&m, Vec3::new(p.point[0], p.point[1], p.point[2]));
            base.point = [q.x, q.y, q.z];
        }
        (base, transform)
    };
    let picks: Vec<_> = crate::viewport::pick_all(app, proj, pos).iter().filter_map(pick).collect();
    if inference || s.request.flatten().is_some() {
        return inferred_viewport(app, ui, proj, pos, click, s, &picks, chain.as_ref());
    }
    let mut best: Option<(f32, GeometrySnap)> = None;
    s.marker = None;
    s.second_marker = None;
    if s.mode == 4 {
        if let Some(edge) = picks.iter().find(|p| {
            matches!(p.kind, GeometryKind::Edge) && s.first.as_ref().is_none_or(|(first, _)| first.body != p.body || first.index != p.index)
        }) {
            if let Some(first) = &s.first {
                let (base, transform) = unpreview(edge);
                match app.session.geometry_intersection_transformed(&first.0, first.1.as_ref(), &base, transform.as_ref()) {
                    Ok(snap) => {
                        s.second_marker = Some(GeometrySnap { point: edge.point, direction: None, label: String::new() });
                        s.error = None;
                        best = Some((0., snap));
                    }
                    Err(e) => {
                        if click {
                            s.error = Some(e.to_string());
                        }
                    }
                }
            } else if click {
                let (base, transform) = unpreview(edge);
                match app.session.geometry_straight_edge(&base) {
                    Ok(_) => {
                        s.first = Some((base, transform));
                        s.first_marker = Some(GeometrySnap {
                            point: edge.point,
                            direction: None,
                            label: solidcraft_ui_text::tr("move.snap.first_selected").to_string(),
                        });
                        s.error = None;
                    }
                    Err(error) => s.error = Some(error.to_string()),
                }
            }
        }
    } else {
        for p in &picks {
            let modes: Vec<_> = match s.mode {
                2 => vec![SnapMode::Midpoint],
                3 => vec![SnapMode::Center],
                1 => vec![SnapMode::Point],
                _ => vec![SnapMode::Point, SnapMode::Midpoint, SnapMode::Center],
            };
            for (mode_index, mode) in modes.into_iter().enumerate() {
                let (base, transform) = unpreview(p);
                let cache_key = format!(
                    "{:?}:{:?}:{}:{:?}:{mode:?}:{:?}",
                    base.kind,
                    base.index,
                    base.body,
                    if matches!(mode, SnapMode::Point) { Some(base.point) } else { None },
                    transform
                );
                let snap = s.cache.get(&cache_key).cloned().or_else(|| {
                    app.session.geometry_snap(&base, mode).ok().map(|mut snap| {
                        if let Some(m) = transform {
                            let point = solidcraft_engine::doc::apply_point(&m, Vec3::new(snap.point[0], snap.point[1], snap.point[2]));
                            snap.point = [point.x, point.y, point.z];
                            if let Some(d) = snap.direction {
                                let d = move_handles::vector(&m, Vec3::new(d[0], d[1], d[2]));
                                snap.direction = Some([d.x, d.y, d.z]);
                            }
                        }
                        if s.cache.len() > 128 {
                            s.cache.clear();
                        }
                        s.cache.insert(cache_key, snap.clone());
                        snap
                    })
                });
                if let Some(snap) = snap {
                    if s.request.flatten().is_some() && snap.direction.is_none() {
                        continue;
                    }
                    if let Some(at) = proj.to_screen(Vec3::new(snap.point[0], snap.point[1], snap.point[2])) {
                        let distance = at.distance(pos);
                        // Explicit modes may choose a centre away from the hovered edge.
                        let score = if s.mode == 0 && distance > 16. {
                            continue;
                        } else {
                            distance - if s.mode == 0 && (mode_index > 0 || matches!(p.kind, GeometryKind::Vertex)) { 16. } else { 0. }
                        };
                        if best.as_ref().is_none_or(|(old, _)| score < *old) {
                            best = Some((score, snap));
                        }
                    }
                }
            }
        }
    }
    s.marker = best.map(|(_, snap)| snap);
    if click && s.marker.is_none() && s.mode != 4 && !picks.is_empty() {
        s.error = Some(solidcraft_ui_text::tr("move.snap.unavailable").to_string());
    }
    let chosen = s.marker.clone();
    put(ui.ctx(), s);
    if click && let Some(snap) = chosen {
        accept(ui.ctx(), &snap);
    }
    true
}

fn same_point(a: [f64; 3], b: [f64; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-7)
}
fn learn(s: &mut State, snap: GeometrySnap, guides: Vec<[f64; 3]>) {
    if let Some(i) = s.inferred.iter().position(|p| same_point(p.snap.point, snap.point) && p.snap.label == snap.label) {
        s.inferred.remove(i);
    }
    s.inferred.push(Inferred { snap, guides });
    if s.inferred.len() > 12 {
        s.inferred.remove(0);
    }
}
fn unplaced_pick(p: &GeometryPick, chain: Option<&(Vec<String>, Mat)>) -> (GeometryPick, Option<Mat>) {
    let transform = chain.filter(|(names, _)| names.contains(&p.body)).map(|(_, m)| *m);
    let mut base = p.clone();
    if let Some(inverse) = transform.and_then(|m| solidcraft_engine::origins::frame_delta(&m, &solidcraft_engine::doc::IDENTITY).ok()) {
        let q = solidcraft_engine::doc::apply_point(&inverse, Vec3::new(p.point[0], p.point[1], p.point[2]));
        base.point = [q.x, q.y, q.z];
    }
    (base, transform)
}
fn inferred_sample(app: &SolveApp, s: &mut State, base: &GeometryPick, transform: Option<Mat>, mode: SnapMode) -> Option<GeometrySnap> {
    let key = format!(
        "inferred:{:?}:{:?}:{}:{mode:?}:{transform:?}:{:?}",
        base.kind,
        base.index,
        base.body,
        if mode == SnapMode::Point { Some(base.point) } else { None }
    );
    if let Some(cached) = s.cache.get(&key) {
        return Some(cached.clone());
    }
    let mut snap = app.session.geometry_snap(base, mode).ok()?;
    if let Some(m) = transform {
        let p = solidcraft_engine::doc::apply_point(&m, Vec3::new(snap.point[0], snap.point[1], snap.point[2]));
        snap.point = [p.x, p.y, p.z];
        if let Some(d) = snap.direction {
            let d = move_handles::vector(&m, Vec3::new(d[0], d[1], d[2]));
            snap.direction = Some([d.x, d.y, d.z]);
        }
    }
    if s.cache.len() >= 128 {
        s.cache.clear();
    }
    s.cache.insert(key, snap.clone());
    Some(snap)
}
/// Hover learns bounded exact targets. Markers remain selectable after leaving their source
/// edge/face, so a circle centre or virtual corner can be picked in otherwise empty space.
fn inferred_viewport(
    app: &SolveApp,
    ui: &egui::Ui,
    proj: &Proj,
    pos: Pos2,
    click: bool,
    mut s: State,
    picks: &[GeometryPick],
    chain: Option<&(Vec<String>, Mat)>,
) -> bool {
    let armed = s.request.is_some();
    let axis = s.request.flatten().is_some();
    // A previously displayed marker wins before hovering underlying geometry can replace it.
    let nearest = |s: &State| {
        s.inferred
            .iter()
            .rev()
            .filter(|p| !axis || p.snap.direction.is_some())
            .filter_map(|p| proj.to_screen(Vec3::new(p.snap.point[0], p.snap.point[1], p.snap.point[2])).map(|at| (at.distance(pos), p.snap.clone())))
            .filter(|(d, _)| *d <= 14.)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, p)| p)
    };
    let retained = nearest(&s);
    let mut primary = None;
    let mut fallback = None;
    if retained.is_none() {
        // Use only the foremost target of each topology kind; deep overlapping bodies must not
        // populate inferred points just because they lie behind the user's hovered geometry.
        let mut kinds = [false; 3];
        for p in picks.iter().take(32) {
            let kind = match p.kind {
                GeometryKind::Vertex => 0,
                GeometryKind::Edge => 1,
                GeometryKind::Face => 2,
            };
            if kinds[kind] {
                continue;
            }
            kinds[kind] = true;
            let (base, transform) = unplaced_pick(p, chain);
            let modes: &[SnapMode] = match p.kind {
                GeometryKind::Vertex => &[SnapMode::Point],
                GeometryKind::Edge => &[SnapMode::Center, SnapMode::Midpoint],
                GeometryKind::Face => &[SnapMode::Center],
            };
            for mode in modes {
                if let Some(snap) = inferred_sample(app, &mut s, &base, transform, *mode) {
                    if primary.is_none() && (!axis || snap.direction.is_some()) {
                        primary = Some(snap.clone());
                    }
                    learn(&mut s, snap, Vec::new());
                }
            }
            if armed && fallback.is_none() {
                fallback = inferred_sample(app, &mut s, &base, transform, SnapMode::Point).filter(|p| !axis || p.direction.is_some());
            }
        }
        if let Some(edge) = picks.iter().find(|p| p.kind == GeometryKind::Edge) {
            let (base, transform) = unplaced_pick(edge, chain);
            let key = format!("{}:{:?}:{transform:?}", base.body, base.index);
            if s.last_line.as_ref() != Some(&key) && app.session.geometry_straight_edge(&base).is_ok() {
                let previous = s.hovered_lines.clone();
                for (first, first_transform, from) in previous.iter().rev() {
                    if first.body == base.body && first.index == base.index {
                        continue;
                    }
                    if let Ok(corner) = app.session.geometry_intersection_transformed(first, first_transform.as_ref(), &base, transform.as_ref()) {
                        learn(&mut s, corner, vec![*from, edge.point]);
                    }
                }
                s.hovered_lines.retain(|(p, _, _)| p.body != base.body || p.index != base.index);
                s.hovered_lines.push((base, transform, edge.point));
                if s.hovered_lines.len() > 3 {
                    s.hovered_lines.remove(0);
                }
                s.last_line = Some(key);
            }
        }
    }
    let nearby = retained.or_else(|| nearest(&s));
    let origin = [s.frame[3][0], s.frame[3][1], s.frame[3][2]];
    let continuing_drag = ui.input(|i| i.pointer.primary_down() && !i.pointer.primary_pressed());
    s.hovering_marker = nearby.as_ref().is_some_and(|p| !same_point(p.point, origin)) && (!continuing_drag || s.hovering_marker);
    s.marker = nearby.clone().or(primary).or(fallback);
    s.first = None;
    s.error = None;
    let chosen = if armed { s.marker.clone() } else { nearby };
    let origin = [s.frame[3][0], s.frame[3][1], s.frame[3][2]];
    // Let the triad's own centre click arm/drag it rather than intercepting a no-op marker.
    let commit = click && chosen.as_ref().is_some_and(|p| armed || !same_point(p.point, origin));
    if commit && s.request.is_none() {
        s.request = Some(None);
    }
    put(ui.ctx(), s);
    if commit && let Some(snap) = chosen {
        accept(ui.ctx(), &snap);
    }
    armed || commit
}

/// Reserve inferred targets when positioning floating numeric controls so their foreground
/// hit areas cannot cover a centre or virtual-corner marker.
pub fn markers_region(ctx: &Context, proj: &Proj) -> Option<egui::Rect> {
    let s = state(ctx)?;
    if s.mode != 0 {
        return None;
    }
    s.inferred
        .iter()
        .filter_map(|p| {
            let p = p.snap.point;
            proj.to_screen(Vec3::new(p[0], p[1], p[2]))
                .filter(|at| proj.rect.contains(*at))
                .map(|at| egui::Rect::from_center_size(at, egui::vec2(36., 36.)))
        })
        .reduce(|a, b| a.union(b))
}

pub fn paint(ctx: &Context, painter: &egui::Painter, proj: &Proj) {
    let Some(s) = state(ctx) else { return };
    let tk = solidcraft_theme::Tokens::get();
    if s.mode == 0 {
        for (i, candidate) in s.inferred.iter().enumerate() {
            let point = candidate.snap.point;
            if let Some(at) = proj.to_screen(Vec3::new(point[0], point[1], point[2])) {
                crate::scenario::publish_handle(&format!("move-snap:candidate:{i}"), at);
                painter.circle_stroke(at, 5., Stroke::new(3.5, tk.hover_edge));
                painter.circle_stroke(at, 5., Stroke::new(2., tk.surface_ink));
                painter.circle_filled(at, 2., tk.surface_ink);
                // Exact virtual intersections display both supporting extensions while inferred.
                for guide in &candidate.guides {
                    if let Some(start) = proj.to_screen(Vec3::new(guide[0], guide[1], guide[2])) {
                        painter.line_segment([start, at], Stroke::new(1., tk.surface_ink));
                    }
                }
            }
        }
    }
    if s.request.is_some() && s.first.is_some() {
        if let Some(first) = &s.first_marker {
            if let Some(at) = proj.to_screen(Vec3::new(first.point[0], first.point[1], first.point[2])) {
                painter.circle_stroke(at, 6., Stroke::new(2., tk.surface_ink));
                painter.text(
                    at + egui::vec2(10., 10.),
                    egui::Align2::LEFT_TOP,
                    &first.label,
                    egui::FontId::proportional(tk.font_small),
                    tk.surface_ink,
                );
            }
        }
        if let Some(corner) = &s.marker {
            if let Some(end) = proj.to_screen(Vec3::new(corner.point[0], corner.point[1], corner.point[2])) {
                for mark in [&s.first_marker, &s.second_marker].into_iter().flatten() {
                    if let Some(start) = proj.to_screen(Vec3::new(mark.point[0], mark.point[1], mark.point[2])) {
                        painter.line_segment([start, end], Stroke::new(1.5, tk.surface_ink));
                    }
                }
            }
        }
    }
    let Some(snap) = s.marker else { return };
    let Some(p) = proj.to_screen(Vec3::new(snap.point[0], snap.point[1], snap.point[2])) else { return };
    let tk = solidcraft_theme::Tokens::get();
    painter.circle_stroke(p, 7., Stroke::new(4., tk.hover_edge));
    painter.circle_stroke(p, 7., Stroke::new(2., tk.surface_ink));
    painter.line_segment([p - egui::vec2(10., 0.), p + egui::vec2(10., 0.)], Stroke::new(1., tk.surface_ink));
    painter.line_segment([p - egui::vec2(0., 10.), p + egui::vec2(0., 10.)], Stroke::new(1., tk.surface_ink));
    painter.text(p + egui::vec2(12., -12.), egui::Align2::LEFT_BOTTOM, &snap.label, egui::FontId::proportional(tk.font_small), tk.surface_ink);
}
