//! Regression for the existing occurrence Move/Copy arrows, independent of clipboard paste.
use crate::{
    context_menu::{self, Target},
    scenario::Harness,
};
use serde_json::json;
use solidcore_record::Vec3;
use solidcraft_engine::geom::prelude::*;
use solidcraft_engine::{Sel, doc};

#[test]
fn occurrence_gizmo_drag_matches_numeric_input_and_preserves_joints_capture_and_undo() {
    let mut h = Harness::new();
    let part = h.app.run("component.create", json!({"name":"Block","grounded":false})).unwrap();
    let first = part["occurrence"].as_u64().unwrap();
    h.app.run("solid.box", json!({"length":10,"width":10,"height":10})).unwrap();
    h.app.run("component.activate", json!({"component":0})).unwrap();
    let second = h.app.run("occurrence.copy", json!({"occurrence":first,"translate":[50,0,0]})).unwrap()["occurrence"].as_u64().unwrap();
    h.app
        .run(
            "joint.create",
            json!({"name":"Fixed peer","type":"rigid","flip":true,"a":{"occurrence":0,"point":[50,0,0]},"b":{"occurrence":second,"point":[50,0,0]}}),
        )
        .unwrap();
    let joints = serde_json::to_value(&h.app.session.doc.assembly.joints).unwrap();
    let original = h.app.session.doc.to_json();
    let initial = doc::apply_point(&h.app.session.occurrence_placement(&[first]).unwrap().world, Vec3::ZERO);
    let peer = doc::apply_point(&h.app.session.occurrence_placement(&[second]).unwrap().world, Vec3::ZERO);
    h.app.cam.target = initial;
    h.app.cam.distance = 100.;
    h.frames(3);
    let target = Target::Occurrence { path: vec![first] };
    let item = context_menu::items(&h.app, &target).into_iter().find(|item| item.id == "ui.moveOccurrence").unwrap();
    context_menu::run_item(&mut h.app, &item, egui::Pos2::ZERO);
    h.frames(3);
    let camera = h.app.cam;
    let start = crate::gizmo::handle_point("x", Vec3::ZERO, 60.).unwrap();
    let end = start + Vec3::X * 12.;
    h.step(&json!({"drag":[{"world":[start.x,start.y,start.z]},{"world":[end.x,end.y,end.z]}],"steps":12})).unwrap();
    h.frames(3);
    let arrow = doc::apply_point(&h.app.session.occurrence_placement(&[first]).unwrap().world, Vec3::ZERO);
    assert!(arrow.x > initial.x + 1., "visible arrow must actually move this occurrence: {initial:?}->{arrow:?}");
    assert_eq!(h.app.cam, camera);
    assert_eq!(h.app.session.doc.to_json(), original, "drag is a pending position");
    assert_eq!(doc::apply_point(&h.app.session.occurrence_placement(&[second]).unwrap().world, Vec3::ZERO), peer);
    assert_eq!(serde_json::to_value(&h.app.session.doc.assembly.joints).unwrap(), joints);
    h.app.run("component.revert_position", json!({"occurrences":[first]})).unwrap();
    h.app.tree.occurrence_move.as_mut().unwrap().translate = [0.; 3];
    h.frames(2);
    let at = h.call("ui.at", json!({"handle":"occurrence-move:distance:0"}));
    h.call("ui.click", json!({"x":at["result"][0],"y":at["result"][1],"double":true}));
    h.frames(2);
    h.step(&json!({"key":"A","cmd":true})).unwrap();
    h.step(&json!({"text":format!("{}",arrow.x-initial.x)})).unwrap();
    h.step(&json!({"key":"Enter"})).unwrap();
    h.frames(3);
    let numeric = doc::apply_point(&h.app.session.occurrence_placement(&[first]).unwrap().world, Vec3::ZERO);
    assert!((numeric - arrow).len() < 1e-6, "numeric and arrow routes must agree: {numeric:?}, {arrow:?}");
    let undo = h.app.session.undo.len();
    h.app.run("component.capture_position", json!({})).unwrap();
    h.app.tree.occurrence_move = None;
    assert_eq!(h.app.session.undo.len(), undo + 1);
    h.app.run("edit.undo", json!({})).unwrap();
    assert_eq!(h.app.session.doc.to_json(), original);
    h.app.run("edit.redo", json!({})).unwrap();
    assert!((doc::apply_point(&h.app.session.occurrence_placement(&[first]).unwrap().world, Vec3::ZERO) - numeric).len() < 1e-6);
    assert_eq!(serde_json::to_value(&h.app.session.doc.assembly.joints).unwrap(), joints);
    assert!(!h.app.session.selection.contains(&Sel::Occurrence { path: vec![second] }));
}

fn handles_fixture() -> (Harness, u64) {
    let mut h = Harness::new();
    h.app.home.open = false;
    let part = h.app.run("component.create", json!({"name":"Handle fixture","grounded":false})).unwrap();
    let id = part["occurrence"].as_u64().unwrap();
    h.app.run("solid.box", json!({"length":10,"width":10,"height":10})).unwrap();
    h.app.run("component.activate", json!({"component":0})).unwrap();
    let mut m = crate::browser::OccMove::new(id);
    m.path = vec![id];
    h.app.tree.occurrence_move = Some(m);
    // Keep geometry targets clear of the right-side Move panel and coordinate dock.
    h.call("ui.camera", json!({"target":[20,20,5],"distance":120,"yaw":0.7,"pitch":0.5,"roll":0}));
    h.frames(3);
    (h, id)
}
fn handles_click(h: &mut Harness, handle: &str) {
    h.step(&json!({"click":{"handle":handle}})).unwrap();
    h.frames(2);
}
fn handles_position(h: &Harness, id: u64) -> Vec3 {
    doc::apply_point(&h.app.session.occurrence_placement(&[id]).unwrap().world, Vec3::ZERO)
}
#[test]
fn component_handles_only_reorients_without_moving_and_local_x_follows_new_frame() {
    let (mut h, id) = handles_fixture();
    let original = h.app.session.doc.to_json();
    let initial = handles_position(&h, id);
    handles_click(&mut h, "occurrence-move:handles-only");
    assert!(h.app.tree.occurrence_move.as_ref().unwrap().handles_only);
    let frame = crate::move_handles::turn(&crate::move_handles::at(Vec3::new(12., 7., 3.)), 2, std::f64::consts::FRAC_PI_2);
    h.app.tree.occurrence_move.as_mut().unwrap().handle_frame = Some(frame);
    h.frames(2);
    assert_eq!(handles_position(&h, id), initial);
    assert_eq!(h.app.session.doc.to_json(), original);
    assert!(h.app.session.pending_moves.is_empty());
    handles_click(&mut h, "occurrence-move:handles-only");
    assert!(!h.app.tree.occurrence_move.as_ref().unwrap().handles_only);
    assert_eq!(handles_position(&h, id), initial, "unchecking must not jump");
    let mut m = h.app.tree.occurrence_move.clone().unwrap();
    m.translate[0] = 14.;
    crate::browser::apply_occurrence_move(&mut h.app, &m);
    h.app.tree.occurrence_move = Some(m);
    assert!((handles_position(&h, id) - initial - Vec3::Y * 14.).len() < 1e-8);
    handles_click(&mut h, "dialog:cancel");
    assert_eq!(handles_position(&h, id), initial);
}

#[test]
fn component_axis_canvas_value_accepts_units_without_moving_part_in_handles_only() {
    let (mut h, id) = handles_fixture();
    handles_click(&mut h, "occurrence-move:handles-only");
    let original = h.app.session.doc.to_json();
    handles_click(&mut h, "move_x");
    assert!(crate::scenario::handle_at("occurrence-move:canvas-value").is_some());
    h.step(&json!({"text":"1 cm + 2 mm"})).unwrap();
    h.frames(3);
    let frame = h.app.tree.occurrence_move.as_ref().unwrap().handle_frame.unwrap();
    assert!((crate::move_handles::origin(&frame) - Vec3::X * 12.).len() < 1e-8);
    assert_eq!(h.app.session.doc.to_json(), original);
    assert!(h.app.session.pending_moves.is_empty());
    assert!((handles_position(&h, id) - Vec3::ZERO).len() < 1e-9);
    handles_click(&mut h, "occurrence-move:canvas-value");
    h.step(&json!({"key":"A","ctrl":true})).unwrap();
    h.step(&json!({"text":"2 cm"})).unwrap();
    h.frames(3);
    let frame = h.app.tree.occurrence_move.as_ref().unwrap().handle_frame.unwrap();
    assert!((crate::move_handles::origin(&frame) - Vec3::X * 20.).len() < 1e-8, "editing replaces the offset without accumulating it");
}

#[test]
fn component_axis_canvas_value_moves_part_and_cancel_restores_placement() {
    let (mut h, id) = handles_fixture();
    let initial = handles_position(&h, id);
    handles_click(&mut h, "move_x");
    assert!(crate::scenario::handle_at("occurrence-move:canvas-value").is_some());
    h.step(&json!({"text":"7 mm"})).unwrap();
    h.frames(3);
    assert!((handles_position(&h, id) - initial - Vec3::X * 7.).len() < 1e-8);
    handles_click(&mut h, "dialog:cancel");
    assert!((handles_position(&h, id) - initial).len() < 1e-8);
}

#[test]
fn component_rotation_canvas_value_keeps_handle_origin_fixed() {
    let (mut h, _) = handles_fixture();
    handles_click(&mut h, "occurrence-move:handles-only");
    let origin = Vec3::new(12., 7., 3.);
    h.app.tree.occurrence_move.as_mut().unwrap().handle_frame = Some(crate::move_handles::at(origin));
    h.frames(3);
    handles_click(&mut h, "move_rz");
    assert!(crate::scenario::handle_at("occurrence-move:canvas-value").is_some());
    h.step(&json!({"text":"90 deg"})).unwrap();
    h.frames(3);
    let frame = h.app.tree.occurrence_move.as_ref().unwrap().handle_frame.unwrap();
    assert!((crate::move_handles::origin(&frame) - origin).len() < 1e-8);
    assert!((crate::move_handles::axes(&frame)[0] - Vec3::Y).len() < 1e-8);
    assert!(h.app.session.pending_moves.is_empty());
}
#[test]
fn component_csys_alignment_matches_full_frame_and_apply_is_scoped() {
    let (mut h, id) = handles_fixture();
    let other = h.app.run("occurrence.copy", json!({"occurrence":id,"translate":[40,0,0]})).unwrap()["occurrence"].as_u64().unwrap();
    h.app.run("occurrence.move", json!({"occurrence":other,"translate":[0,9,0],"capture":false})).unwrap();
    let other_pending = h.app.session.pending_moves[&other];
    let source = crate::move_handles::turn(&crate::move_handles::at(Vec3::new(10., 20., 30.)), 2, 0.7);
    h.app.tree.occurrence_move.as_mut().unwrap().handle_frame = Some(source);
    h.app.tree.occurrence_move.as_mut().unwrap().target_frame = "world".into();
    h.frames(2);
    let old = h.app.session.occurrence_placement(&[id]).unwrap().world;
    let expected = doc::mat_mul(&solidcraft_engine::origins::frame_delta(&source, &doc::IDENTITY).unwrap(), &old);
    handles_click(&mut h, "move-handles:align");
    let actual = h.app.session.occurrence_placement(&[id]).unwrap().world;
    for i in 0..4 {
        for j in 0..4 {
            assert!((actual[i][j] - expected[i][j]).abs() < 1e-8);
        }
    }
    handles_click(&mut h, "dialog:apply");
    assert!(h.app.tree.occurrence_move.is_some());
    assert!(!h.app.session.pending_moves.contains_key(&id));
    assert_eq!(h.app.session.pending_moves[&other], other_pending);
    let committed = handles_position(&h, id);
    let mut m = h.app.tree.occurrence_move.clone().unwrap();
    m.translate[0] = 5.;
    crate::browser::apply_occurrence_move(&mut h.app, &m);
    h.app.tree.occurrence_move = Some(m);
    handles_click(&mut h, "dialog:cancel");
    assert!((handles_position(&h, id) - committed).len() < 1e-8);
    assert_eq!(h.app.session.pending_moves[&other], other_pending);
}
#[test]
fn component_stale_csys_is_retained_and_refuses_alignment() {
    let (mut h, id) = handles_fixture();
    let initial = handles_position(&h, id);
    h.app.tree.occurrence_move.as_mut().unwrap().target_frame = "joint:deleted:123".into();
    h.frames(3);
    assert_eq!(h.app.tree.occurrence_move.as_ref().unwrap().target_frame, "joint:deleted:123");
    handles_click(&mut h, "move-handles:align");
    assert_eq!(handles_position(&h, id), initial);
    assert!(h.app.session.pending_moves.is_empty());
}
#[test]
fn component_rotation_precision_covers_tiny_and_half_turns() {
    for axis in [Vec3::X, Vec3::Y, Vec3::new(1., -2., 3.).normalized().unwrap()] {
        for angle in [1e-13, 1e-11, 1e-10, 1e-9, 1e-6, 0.7, std::f64::consts::PI] {
            let r = crate::gizmo::rotation(axis, angle);
            let (a, t) = crate::gizmo::axis_angle(&r);
            let back = crate::gizmo::rotation(a, t);
            for i in 0..3 {
                for j in 0..3 {
                    assert!((r[i][j] - back[i][j]).abs() < 1e-14, "angle={angle}, axis={axis:?}");
                }
            }
        }
    }
}
#[test]
fn component_preferences_restore_and_respect_disabled_memory() {
    let (mut h, id) = handles_fixture();
    h.app.preferences.remember_dialog_values = true;
    {
        let m = h.app.tree.occurrence_move.as_mut().unwrap();
        m.handles_only = true;
        m.handle_reference = "world".into();
        m.target_frame = "world".into();
    }
    crate::browser_prompts::confirm_occurrence_move(&mut h.app, crate::ui_kit::Action::Apply);
    assert_eq!(h.app.preferences.command_dialog_values["occurrence.move"]["confirmation"], "apply");
    let mut reopened = crate::browser::OccMove::new(id);
    reopened.path = vec![id];
    h.app.tree.occurrence_move = Some(reopened);
    h.frames(2);
    assert!(h.app.tree.occurrence_move.as_ref().unwrap().handles_only);
    h.app.preferences.remember_dialog_values = false;
    let saved = h.app.preferences.command_dialog_values["occurrence.move"].clone();
    let mut reopened = crate::browser::OccMove::new(id);
    reopened.path = vec![id];
    h.app.tree.occurrence_move = Some(reopened);
    h.frames(2);
    assert!(!h.app.tree.occurrence_move.as_ref().unwrap().handles_only);
    crate::browser_prompts::confirm_occurrence_move(&mut h.app, crate::ui_kit::Action::Ok);
    assert_eq!(h.app.preferences.command_dialog_values["occurrence.move"], saved);
    assert!(h.app.tree.occurrence_move.is_none());
}
#[test]
fn component_visible_arrow_moves_handles_only_then_middle_apply_keeps_tool() {
    let (mut h, id) = handles_fixture();
    let initial = handles_position(&h, id);
    handles_click(&mut h, "occurrence-move:handles-only");
    let start = crate::gizmo::handle_point("x", Vec3::ZERO, 60.).unwrap();
    let end = start + Vec3::X * 12.;
    h.step(&json!({"drag":[{"world":[start.x,start.y,start.z]},{"world":[end.x,end.y,end.z]}],"steps":12})).unwrap();
    h.frames(2);
    let frame = h.app.tree.occurrence_move.as_ref().unwrap().handle_frame.unwrap();
    assert!(crate::move_handles::origin(&frame).x > 1., "visible arrow must move handles");
    assert_eq!(handles_position(&h, id), initial);
    assert!(h.app.session.pending_moves.is_empty());
    handles_click(&mut h, "occurrence-move:handles-only");
    let mut m = h.app.tree.occurrence_move.clone().unwrap();
    m.translate[0] = 7.;
    crate::browser::apply_occurrence_move(&mut h.app, &m);
    h.app.tree.occurrence_move = Some(m);
    h.call("ui.move", json!({"x":500,"y":700}));
    h.frames(2);
    h.call("ui.click", json!({"x":500,"y":700,"button":"middle"}));
    h.frames(2);
    assert!(h.app.tree.occurrence_move.is_some());
    assert!(!h.app.session.pending_moves.contains_key(&id));
    assert!((handles_position(&h, id) - initial - Vec3::X * 7.).len() < 1e-8);
}
#[test]
fn component_cancel_restores_preview_that_existed_before_tool_opened() {
    let (mut h, id) = handles_fixture();
    h.app.tree.occurrence_move = None;
    h.app.run("occurrence.move", json!({"occurrence":id,"translate":[3,4,5],"capture":false})).unwrap();
    let before = h.app.session.pending_moves[&id];
    let mut m = crate::browser::OccMove::new(id);
    m.path = vec![id];
    h.app.tree.occurrence_move = Some(m);
    h.frames(2);
    handles_click(&mut h, "occurrence-move:handles-only");
    h.app.tree.occurrence_move.as_mut().unwrap().handle_frame = Some(crate::move_handles::at(Vec3::new(20., 30., 40.)));
    handles_click(&mut h, "occurrence-move:handles-only");
    let mut m = h.app.tree.occurrence_move.clone().unwrap();
    m.translate[1] = 8.;
    crate::browser::apply_occurrence_move(&mut h.app, &m);
    h.app.tree.occurrence_move = Some(m);
    handles_click(&mut h, "dialog:cancel");
    assert_eq!(h.app.session.pending_moves[&id], before);
}

#[test]
fn component_geometry_origin_picker_uses_visible_exact_vertex() {
    let (mut h, id) = handles_fixture();
    h.app.run("occurrence.copy", json!({"occurrence":id,"translate":[30,30,0]})).unwrap();
    h.app.cam.distance = 160.;
    h.frames(3);
    handles_click(&mut h, "occurrence-move:handles-only");
    handles_click(&mut h, "move-snap:origin");
    assert!(crate::move_handle_snap::active_owner(&h.ctx).is_some());
    h.step(&json!({"click":{"world":[40.,40.,10.]}})).unwrap();
    h.frames(3);
    let frame = h.app.tree.occurrence_move.as_ref().unwrap().handle_frame.unwrap();
    let at = crate::move_handles::origin(&frame);
    assert!((at - Vec3::new(40., 40., 10.)).len() < 1e-7, "visible vertex pick returned {at:?}");
    assert!(h.app.session.pending_moves.is_empty());
    assert!((handles_position(&h, id) - Vec3::ZERO).len() < 1e-9);
}

#[test]
fn component_center_click_arms_snap_and_center_drag_snaps_to_geometry() {
    let (mut h, id) = handles_fixture();
    h.app.run("occurrence.copy", json!({"occurrence":id,"translate":[30,30,0]})).unwrap();
    h.app.cam.distance = 160.;
    h.frames(3);
    handles_click(&mut h, "occurrence-move:handles-only");
    h.app.tree.occurrence_move.as_mut().unwrap().handle_frame = Some(crate::move_handles::at(Vec3::new(25., 25., 0.)));
    h.frames(3);
    h.step(&json!({"click":{"handle":"move_ball"}})).unwrap();
    h.frames(2);
    assert!(crate::move_handle_snap::active_owner(&h.ctx).is_some(), "centre click arms geometry picker");
    handles_click(&mut h, "move-snap:cancel");
    let start = crate::gizmo::handle_point("ball", Vec3::ZERO, 0.).unwrap();
    h.step(&json!({"drag":[{"world":[start.x,start.y,start.z]},{"world":[40.,40.,10.]}],"steps":20})).unwrap();
    h.frames(3);
    let frame = h.app.tree.occurrence_move.as_ref().unwrap().handle_frame.unwrap();
    let at = crate::move_handles::origin(&frame);
    assert!((at - Vec3::new(40., 40., 10.)).len() < 1e-7, "centre drag selected {at:?}");
    assert!(h.app.session.pending_moves.is_empty());
}

#[test]
fn component_virtual_corner_mode_arms_two_edge_pick_and_keeps_geometry_stationary() {
    let (mut h, id) = handles_fixture();
    h.app.run("occurrence.copy", json!({"occurrence":id,"translate":[30,30,0]})).unwrap();
    h.app.cam.distance = 160.;
    h.frames(3);
    handles_click(&mut h, "occurrence-move:handles-only");
    handles_click(&mut h, "move-snap:mode");
    h.step(&json!({"widget":"Virtual intersection"})).unwrap();
    h.frames(3);
    assert!(crate::move_handle_snap::active(&h.ctx), "selecting the mode must arm the picker");
    h.step(&json!({"click":{"world":[5.,10.,10.]}})).unwrap();
    h.frames(3);
    assert!(crate::move_handle_snap::active(&h.ctx), "first edge must not commit a move");
    h.step(&json!({"click":{"world":[40.,35.,10.]}})).unwrap();
    h.frames(3);
    assert!(!crate::move_handle_snap::active(&h.ctx), "second edge must complete the virtual corner");
    let frame = h.app.tree.occurrence_move.as_ref().unwrap().handle_frame.unwrap();
    let at = crate::move_handles::origin(&frame);
    assert!((at - Vec3::new(40., 10., 10.)).len() < 1e-7, "extended lines meet in empty space: {at:?}");
    assert!(h.app.session.pending_moves.is_empty());
    assert!((handles_position(&h, id) - Vec3::ZERO).len() < 1e-9);
}

#[test]
fn component_face_center_picker_snaps_cylinder_and_cone_without_moving_part() {
    for (command, params, surface_point) in [
        ("solid.cylinder", json!({"diameter":10,"height":20,"base":[35,35,0]}), [40., 35., 10.]),
        ("solid.cone", json!({"radius":8,"top_radius":2,"height":20,"base":[35,35,0]}), [40., 35., 10.]),
    ] {
        let (mut h, id) = handles_fixture();
        h.app.run(command, params).unwrap();
        h.app.cam.distance = 160.;
        h.frames(3);
        handles_click(&mut h, "occurrence-move:handles-only");
        handles_click(&mut h, "move-snap:mode");
        h.step(&json!({"widget":"Circle / face center"})).unwrap();
        h.frames(3);
        assert!(crate::move_handle_snap::active(&h.ctx));
        h.step(&json!({"click":{"world":surface_point}})).unwrap();
        h.frames(3);
        assert!(!crate::move_handle_snap::active(&h.ctx), "face centre pick must complete for {command}");
        let frame = h.app.tree.occurrence_move.as_ref().unwrap().handle_frame.unwrap();
        let at = crate::move_handles::origin(&frame);
        assert!((at - Vec3::new(35., 35., 10.)).len() < 1e-7, "axis midpoint for {command}: {at:?}");
        assert!(h.app.session.pending_moves.is_empty());
        assert!((handles_position(&h, id) - Vec3::ZERO).len() < 1e-9);
        handles_click(&mut h, "occurrence-move:handles-only");
        handles_click(&mut h, "move-snap:origin");
        h.step(&json!({"click":{"world":surface_point}})).unwrap();
        h.frames(3);
        assert!(
            (handles_position(&h, id) - Vec3::ZERO).len() < 1e-9,
            "toggling handles-only must preserve Center mode, not move to the face surface"
        );
    }
}

#[test]
fn component_axis_handle_click_aligns_planar_face_even_in_center_snap_mode() {
    let (mut h, id) = handles_fixture();
    h.app.run("solid.box", json!({"length":10,"width":10,"height":10,"corner":[50,40,0]})).unwrap();
    h.call("ui.camera", json!({"target":[30,20,5],"distance":180,"yaw":0.7,"pitch":0.5,"roll":0}));
    h.frames(3);
    handles_click(&mut h, "occurrence-move:handles-only");
    handles_click(&mut h, "move-snap:mode");
    h.step(&json!({"widget":"Circle / face center"})).unwrap();
    h.frames(2);
    let before = h.app.session.doc.to_json();
    let origin = crate::browser::occurrence_triad_model(&h.app).unwrap().center;
    handles_click(&mut h, "move_x");
    h.step(&json!({"click":{"world":[55.,45.,10.]}})).unwrap();
    h.frames(3);
    let t = crate::browser::occurrence_triad_model(&h.app).unwrap();
    assert!((t.axes[0] - Vec3::Z).len() < 1e-8, "selected X aligns with planar face normal: {:?}", t.axes);
    assert!((t.center - origin).len() < 1e-8);
    assert_eq!(h.app.session.doc.to_json(), before);
    assert!(h.app.session.pending_moves.is_empty());
    assert!((handles_position(&h, id) - Vec3::ZERO).len() < 1e-9);
}

fn hover_move_target(h: &mut crate::scenario::Harness, point: [f64; 3]) {
    h.step(&json!({"move":{"world":point}})).unwrap();
    h.frames(12);
}
fn assert_move_candidate_at(h: &mut crate::scenario::Harness, point: [f64; 3]) {
    let screen = h.call("ui.worldToScreen", json!({"points":[point]}));
    let target = &screen["result"][0];
    let x = target[0].as_f64().unwrap();
    let y = target[1].as_f64().unwrap();
    let handles = h.call("ui.handles", json!({"prefix":"move-snap:candidate:"}));
    assert!(
        handles["result"]
            .as_array()
            .unwrap()
            .iter()
            .any(|handle| { (handle["x"].as_f64().unwrap() - x).hypot(handle["y"].as_f64().unwrap() - y) < 2. }),
        "No visible inferred marker at {point:?}: {handles}"
    );
}
#[test]
fn component_automatic_hover_exposes_clickable_face_and_circle_centers() {
    for active in [false, true] {
        for (hover, expected) in [([40., 35., 10.], [35., 35., 10.]), ([40., 35., 20.], [35., 35., 20.])] {
            let (mut h, id) = handles_fixture();
            h.app.run("solid.cylinder", json!({"diameter":10,"height":20,"base":[35,35,0]})).unwrap();
            h.app.cam.distance = 160.;
            h.frames(3);
            handles_click(&mut h, "occurrence-move:handles-only");
            if active {
                handles_click(&mut h, "move-snap:origin");
            }
            let before = h.app.session.doc.to_json();
            hover_move_target(&mut h, hover);
            assert_move_candidate_at(&mut h, expected);
            hover_move_target(&mut h, expected);
            assert_move_candidate_at(&mut h, expected);
            assert_eq!(h.app.session.doc.to_json(), before, "hover must not alter geometry");
            assert!(h.app.session.pending_moves.is_empty());
            h.step(&json!({"click":{"world":expected}})).unwrap();
            h.frames(3);
            let frame = h.app.tree.occurrence_move.as_ref().unwrap().handle_frame.unwrap();
            let at = crate::move_handles::origin(&frame);
            assert!((at - Vec3::new(expected[0], expected[1], expected[2])).len() < 1e-8, "automatic face/circle marker {active}: {at:?}");
            assert!((handles_position(&h, id) - Vec3::ZERO).len() < 1e-9);
            assert!(h.app.session.pending_moves.is_empty());
        }
    }
}
#[test]
fn component_automatic_hover_infers_virtual_corner_without_edge_clicks() {
    let (mut h, id) = handles_fixture();
    h.app.run("occurrence.copy", json!({"occurrence":id,"translate":[30,30,0]})).unwrap();
    h.app.cam.distance = 160.;
    h.frames(3);
    handles_click(&mut h, "occurrence-move:handles-only");
    let before = h.app.session.doc.to_json();
    hover_move_target(&mut h, [5., 10., 10.]);
    hover_move_target(&mut h, [40., 35., 10.]);
    assert_move_candidate_at(&mut h, [40., 10., 10.]);
    hover_move_target(&mut h, [40., 10., 10.]);
    assert_move_candidate_at(&mut h, [40., 10., 10.]);
    assert_eq!(h.app.session.doc.to_json(), before);
    assert!(h.app.session.pending_moves.is_empty());
    h.step(&json!({"click":{"world":[40.,10.,10.]}})).unwrap();
    h.frames(3);
    let frame = h.app.tree.occurrence_move.as_ref().unwrap().handle_frame.unwrap();
    assert!((crate::move_handles::origin(&frame) - Vec3::new(40., 10., 10.)).len() < 1e-8);
    assert!((handles_position(&h, id) - Vec3::ZERO).len() < 1e-9);
}
#[test]
fn component_automatic_hover_markers_can_be_disabled() {
    let (mut h, _) = handles_fixture();
    h.app.preferences.move_inference_markers = false;
    h.app.run("solid.cylinder", json!({"diameter":10,"height":20,"base":[35,35,0]})).unwrap();
    h.app.cam.distance = 160.;
    h.frames(3);
    handles_click(&mut h, "occurrence-move:handles-only");
    hover_move_target(&mut h, [40., 35., 20.]);
    let handles = h.call("ui.handles", json!({"prefix":"move-snap:candidate:"}));
    assert!(handles["result"].as_array().unwrap().is_empty(), "legacy mode must not expose inferred targets");
    assert!(!crate::move_handle_snap::active(&h.ctx));
}

#[test]
fn inferred_center_over_gizmo_arrow_commits_once_without_arming_axis() {
    let (mut h, id) = handles_fixture();
    h.app.cam.distance = 250.;
    h.frames(3);
    handles_click(&mut h, "occurrence-move:handles-only");
    let target = crate::gizmo::handle_point("x", Vec3::ZERO, 60.).unwrap();
    h.app.run("solid.cylinder", json!({"diameter":8,"height":10,"base":[target.x,target.y,target.z-10.]})).unwrap();
    h.frames(3);
    hover_move_target(&mut h, [target.x + 4., target.y, target.z]);
    assert_move_candidate_at(&mut h, [target.x, target.y, target.z]);
    hover_move_target(&mut h, [target.x, target.y, target.z]);
    h.step(&json!({"click":{"world":[target.x,target.y,target.z]}})).unwrap();
    h.frames(3);
    let frame = h.app.tree.occurrence_move.as_ref().unwrap().handle_frame.unwrap();
    assert!((crate::move_handles::origin(&frame) - target).len() < 1e-8);
    assert!(!crate::move_handle_snap::active(&h.ctx), "one marker click must not also arm the overlapping axis handle");
    assert!(h.app.session.pending_moves.is_empty());
    assert!((handles_position(&h, id) - Vec3::ZERO).len() < 1e-9);
}
