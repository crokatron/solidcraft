//! The design's first part is grounded; dragging a free part moves it (#2804).
use serde_json::{Value, json};

use crate::Session;

fn run(s: &mut Session, command: &str, p: Value) -> Value {
    s.execute(command, &p).unwrap_or_else(|e| panic!("{command} {p}: {e}"))
}

/// A new top-level part holding a 10 mm cube at `corner` (activates the design first, so the
/// part goes into it): (component, occurrence).
fn part(s: &mut Session, name: &str, corner: [f64; 3]) -> (u64, u64) {
    run(s, "component.activate", json!({"component": "root"}));
    let c = run(s, "component.create", json!({"name": name}));
    run(s, "solid.box", json!({"length": 10, "width": 10, "height": 10, "corner": corner}));
    run(s, "component.activate", json!({"component": "root"}));
    (c["component"].as_u64().unwrap(), c["occurrence"].as_u64().unwrap())
}

fn grounded(s: &Session, occurrence: u64) -> bool {
    s.doc.occurrences.iter().find(|o| o.id == occurrence).unwrap().grounded
}

/// Where the placement `path` puts its origin, in world coordinates (pending moves included).
fn origin(s: &Session, path: &[u64]) -> [f64; 3] {
    let w = s.occurrence_placement(path).unwrap().world;
    [w[3][0], w[3][1], w[3][2]]
}

fn near(a: [f64; 3], b: [f64; 3]) {
    assert!(a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9), "{a:?} != {b:?}");
}

#[test]
fn the_first_top_level_part_is_grounded_and_later_parts_and_copies_are_free() {
    let mut s = Session::default();
    let (a, first) = part(&mut s, "A", [0.0, 0.0, 0.0]);
    let created = s.doc.occurrences.iter().find(|o| o.id == first).cloned().unwrap();
    assert!(created.grounded, "the first part is grounded");
    let (_, second) = part(&mut s, "B", [20.0, 0.0, 0.0]);
    assert!(!grounded(&s, second), "a later part starts free");
    let instance = run(&mut s, "occurrence.copy", json!({"occurrence": first, "translate": [0, 20, 0]}))["occurrence"].as_u64().unwrap();
    assert!(!grounded(&s, instance), "Paste (Instance) of the grounded part is free");
    let copy = run(&mut s, "component.paste_new", json!({"component": a, "translate": [0, 40, 0]}));
    let copy = s.doc.occurrence_of(copy["component"].as_u64().unwrap()).unwrap().id;
    assert!(!grounded(&s, copy), "Paste New of the grounded part is free");
    // A part made inside another one is placed in it, not in the design: free.
    run(&mut s, "component.activate", json!({"component": a}));
    let nested = run(&mut s, "component.create", json!({"name": "Inner"}))["occurrence"].as_u64().unwrap();
    assert!(!grounded(&s, nested));
    // Several parts may be grounded, or none.
    run(&mut s, "occurrence.ground", json!({"occurrence": second, "grounded": true}));
    assert!(grounded(&s, first) && grounded(&s, second));
    for o in [first, second] {
        run(&mut s, "occurrence.ground", json!({"occurrence": o, "grounded": false}));
    }
    assert!(!s.doc.occurrences.iter().any(|o| o.grounded));
}

#[test]
fn components_from_bodies_ground_only_the_first_part() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 10, "width": 10, "height": 10, "body_name": "One"}));
    run(&mut s, "solid.box", json!({"length": 10, "width": 10, "height": 10, "corner": [20, 0, 0], "body_name": "Two", "operation": "new"}));
    let made = run(&mut s, "component.from_bodies", json!({"bodies": ["One", "Two"]}));
    let occurrence = |i: usize| s.doc.occurrence_of(made["components"][i]["component"].as_u64().unwrap()).unwrap().id;
    let (one, two) = (occurrence(0), occurrence(1));
    assert!(grounded(&s, one) && !grounded(&s, two));
}

#[test]
fn ungrounded_designs_stay_ungrounded_through_new_parts_and_reopening() {
    let mut s = Session::default();
    let (_, first) = part(&mut s, "A", [0.0, 0.0, 0.0]);
    run(&mut s, "occurrence.ground", json!({"occurrence": first, "grounded": false}));
    let (_, second) = part(&mut s, "B", [20.0, 0.0, 0.0]);
    assert!(!grounded(&s, first) && !grounded(&s, second), "a part added later grounds nothing");
    let back = solidcraft_doc::Document::from_json(&s.doc.to_json()).unwrap();
    assert!(!back.occurrences.iter().any(|o| o.grounded), "opening a design grounds nothing");
    // With every part gone, the next one is the design's first part again.
    for c in ["A", "B"] {
        run(&mut s, "component.delete", json!({"component": c}));
    }
    let (_, again) = part(&mut s, "C", [0.0, 0.0, 0.0]);
    assert!(grounded(&s, again));
}

#[test]
fn dragging_moves_a_free_part_by_the_grabbed_point_in_one_undo_step() {
    let mut s = Session::default();
    let (_, base) = part(&mut s, "Base", [0.0, 0.0, 0.0]);
    let (_, free) = part(&mut s, "Free", [20.0, 0.0, 0.0]);
    let undo = s.undo.len();
    let r = run(&mut s, "occurrence.drag", json!({"occurrence": free, "at": [25, 5, 10], "point": [35, 10, 10]}));
    assert_eq!(r["moved"], json!([free]));
    near(origin(&s, &[free]), [10.0, 5.0, 0.0]);
    assert_eq!(s.undo.len(), undo + 1);
    assert_eq!(s.undo.last().map(|u| u.label.as_str()), Some("Drag Occurrence"));
    // Saved and opened again, it is where it was dragged.
    let back = solidcraft_doc::Document::from_json(&s.doc.to_json()).unwrap();
    let t = back.occurrences.iter().find(|o| o.id == free).unwrap().transform;
    near([t[3][0], t[3][1], t[3][2]], [10.0, 5.0, 0.0]);
    run(&mut s, "edit.undo", json!({}));
    near(origin(&s, &[free]), [0.0, 0.0, 0.0]);
    // A grounded part stays where it is.
    let e = s.execute("occurrence.drag", &json!({"occurrence": base, "at": [0, 0, 0], "point": [5, 0, 0]})).unwrap_err().to_string();
    assert!(e.contains("grounded"), "{e}");
    near(origin(&s, &[base]), [0.0, 0.0, 0.0]);
}

#[test]
fn a_drag_captures_its_part_and_leaves_other_pending_moves_pending() {
    let mut s = Session::default();
    part(&mut s, "Base", [0.0, 0.0, 0.0]);
    let (_, a) = part(&mut s, "A", [20.0, 0.0, 0.0]);
    let (_, b) = part(&mut s, "B", [40.0, 0.0, 0.0]);
    run(&mut s, "occurrence.move", json!({"occurrence": a, "translate": [0, 7, 0], "capture": false}));
    run(&mut s, "occurrence.move", json!({"occurrence": b, "translate": [0, 9, 0], "capture": false}));
    run(&mut s, "occurrence.drag", json!({"occurrence": a, "at": [0, 0, 0], "point": [3, 0, 0]}));
    assert!(!s.pending_moves.contains_key(&a), "the dragged part's position is captured");
    let t = s.doc.occurrences.iter().find(|o| o.id == a).unwrap().transform;
    near([t[3][0], t[3][1], t[3][2]], [3.0, 7.0, 0.0]);
    assert!(s.pending_moves.contains_key(&b), "another part's move stays pending");
    near(origin(&s, &[b]), [0.0, 9.0, 0.0]);
    run(&mut s, "edit.undo", json!({}));
    near(origin(&s, &[a]), [0.0, 7.0, 0.0]);
    assert!(s.pending_moves.contains_key(&a), "undo gives the pending move back");
}

#[test]
fn dragging_uses_the_exact_path_of_a_repeated_placement() {
    let mut s = Session::default();
    part(&mut s, "Base", [0.0, 0.0, 0.0]);
    let (asm, first) = part(&mut s, "Asm", [0.0, 0.0, 0.0]);
    run(&mut s, "occurrence.move", json!({"occurrence": first, "axis": [0, 0, 1], "angle": "90 deg"}));
    run(&mut s, "component.activate", json!({"component": asm}));
    let leaf = run(&mut s, "component.create", json!({"name": "Leaf"}))["occurrence"].as_u64().unwrap();
    run(&mut s, "solid.box", json!({"length": 2, "width": 2, "height": 2}));
    run(&mut s, "component.activate", json!({"component": "root"}));
    let second = run(&mut s, "occurrence.copy", json!({"occurrence": first, "translate": [100, 0, 0]}))["occurrence"].as_u64().unwrap();
    // The second instance of the assembly moves alone.
    run(&mut s, "occurrence.drag", json!({"path": [second], "at": [100, 0, 0], "point": [100, 30, 0]}));
    near(origin(&s, &[second]), [100.0, 30.0, 0.0]);
    near(origin(&s, &[first]), [0.0, 0.0, 0.0]);
    // Its leaf, a definition shared by both instances, moves in world X through that path's
    // rotated frame, and so in both.
    let before = origin(&s, &[first, leaf]);
    run(&mut s, "occurrence.drag", json!({"path": [second, leaf], "at": [100, 30, 0], "point": [110, 30, 0]}));
    near(origin(&s, &[second, leaf]), [110.0, 30.0, 0.0]);
    near(origin(&s, &[first, leaf]), [before[0] + 10.0, before[1], before[2]]);
    // The leaf alone, without the path of the instance it is in, is ambiguous.
    let e = s.execute("occurrence.drag", &json!({"occurrence": leaf, "at": [0, 0, 0], "point": [1, 0, 0]})).unwrap_err().to_string();
    assert!(e.contains("path"), "{e}");
}

#[test]
fn joints_hold_a_part_to_something_grounded_and_carry_a_free_chain_along() {
    let mut s = Session::default();
    let (_, base) = part(&mut s, "Base", [0.0, 0.0, 0.0]);
    let (_, arm) = part(&mut s, "Arm", [0.0, 0.0, 10.0]);
    let at = |o: u64, z: f64| json!({"occurrence": o, "point": [0, 0, z], "z": [0, 0, 1]});
    run(&mut s, "joint.create", json!({"type": "revolute", "a": at(base, 10.0), "b": at(arm, 10.0), "flip": true, "name": "Hinge"}));
    let e = s.execute("occurrence.drag", &json!({"occurrence": arm, "at": [0, 0, 0], "point": [5, 0, 0]})).unwrap_err().to_string();
    assert!(e.contains("grounded") && e.contains("Hinge"), "{e}");
    // Two parts jointed to each other and to nothing grounded move together.
    let (_, x) = part(&mut s, "X", [50.0, 0.0, 0.0]);
    let (_, y) = part(&mut s, "Y", [50.0, 0.0, 10.0]);
    run(&mut s, "joint.create", json!({"type": "rigid", "a": at(x, 10.0), "b": at(y, 10.0), "flip": true, "name": "Weld"}));
    let (x0, y0) = (origin(&s, &[x]), origin(&s, &[y]));
    let r = run(&mut s, "occurrence.drag", json!({"occurrence": y, "at": [0, 0, 0], "point": [0, 25, 0]}));
    assert_eq!(r["moved"].as_array().map(Vec::len), Some(2), "{r}");
    near(origin(&s, &[x]), [x0[0], x0[1] + 25.0, x0[2]]);
    near(origin(&s, &[y]), [y0[0], y0[1] + 25.0, y0[2]]);
    // Suppressed, the joint holds nothing.
    run(&mut s, "joint.edit", json!({"joint": "Hinge", "suppressed": true}));
    run(&mut s, "occurrence.drag", json!({"occurrence": arm, "at": [0, 0, 0], "point": [5, 0, 0]}));
}

#[test]
fn a_drag_and_a_world_move_leave_the_same_design() {
    // Whatever follows a moved part (projections, joints, Properties) follows a dragged one.
    let build = || {
        let mut s = Session::default();
        part(&mut s, "Base", [0.0, 0.0, 0.0]);
        let (_, free) = part(&mut s, "Free", [20.0, 0.0, 0.0]);
        run(&mut s, "sketch.create", json!({"plane": "XY"}));
        run(&mut s, "sketch.project", json!({"refs": [{"edge": [25, 0, 0]}]}));
        run(&mut s, "sketch.finish", json!({}));
        (s, free)
    };
    let (mut dragged, free) = build();
    let (mut moved, _) = build();
    run(&mut dragged, "occurrence.drag", json!({"occurrence": free, "at": [25, 0, 0], "point": [25, 15, 0]}));
    run(&mut moved, "occurrence.move", json!({"occurrence": free, "space": "world", "translate": [0, 15, 0]}));
    assert_eq!(dragged.doc.to_json(), moved.doc.to_json());
    let shown = |s: &Session| format!("{:?}", s.world_state().unwrap().sketches.iter().map(|k| &k.sketch.points).collect::<Vec<_>>());
    assert_eq!(shown(&dragged), shown(&moved));
}

#[test]
fn bad_drag_parameters_are_refused() {
    let mut s = Session::default();
    part(&mut s, "Base", [0.0, 0.0, 0.0]);
    let (_, free) = part(&mut s, "Free", [20.0, 0.0, 0.0]);
    for p in [
        json!({}),
        json!({"occurrence": free}),
        json!({"occurrence": free, "at": [0, 0, 0]}),
        json!({"occurrence": free, "at": [0, 0, 0], "point": [1e300, 0, 0]}),
        json!({"occurrence": free, "at": [0, 0, "x"], "point": [0, 0, 0]}),
        json!({"path": [], "at": [0, 0, 0], "point": [1, 0, 0]}),
        json!({"path": [free, free], "at": [0, 0, 0], "point": [1, 0, 0]}),
        json!({"path": [999_999], "at": [0, 0, 0], "point": [1, 0, 0]}),
        json!({"occurrence": "nothing", "at": [0, 0, 0], "point": [1, 0, 0]}),
    ] {
        assert!(s.execute("occurrence.drag", &p).is_err(), "{p}");
    }
    near(origin(&s, &[free]), [0.0, 0.0, 0.0]);
}

#[test]
fn moving_a_grounded_part_needs_saying_so_and_it_stays_grounded() {
    let mut s = Session::default();
    let (_, base) = part(&mut s, "Base", [0.0, 0.0, 0.0]);
    let e = s.execute("occurrence.move", &json!({"occurrence": base, "translate": [5, 0, 0]})).unwrap_err().to_string();
    assert!(e.contains("is grounded") && e.contains("occurrence.ground"), "{e}");
    run(&mut s, "occurrence.move", json!({"occurrence": base, "translate": [5, 0, 0], "move_grounded": true}));
    near(origin(&s, &[base]), [5.0, 0.0, 0.0]);
    assert!(grounded(&s, base));
}

/// The solver uses definitions' first paths; a drag through a rotated instance must carry
/// that local change to its jointed peers, including peers that are themselves shared.
#[test]
fn shared_joint_chains_follow_the_selected_frame_and_survive_solving() {
    let mut s = Session::default();
    part(&mut s, "Base", [0.0, 0.0, 0.0]);
    let asm = run(&mut s, "component.create", json!({"name": "Assembly"}));
    let first = asm["occurrence"].as_u64().unwrap();
    let a = run(&mut s, "component.create", json!({"name": "A"}))["occurrence"].as_u64().unwrap();
    run(&mut s, "solid.box", json!({"length": 4, "width": 4, "height": 4}));
    run(&mut s, "component.activate", json!({"component": asm["component"]}));
    let b = run(&mut s, "component.create", json!({"name": "B"}))["occurrence"].as_u64().unwrap();
    run(&mut s, "solid.box", json!({"length": 4, "width": 4, "height": 4, "corner": [10, 0, 0]}));
    run(&mut s, "component.activate", json!({"component": "root"}));
    let at = |o| json!({"occurrence": o, "point": [0, 0, 0], "z": [0, 0, 1]});
    run(&mut s, "joint.create", json!({"type": "revolute", "a": at(a), "b": at(b), "flip": true}));
    let second = run(&mut s, "occurrence.copy", json!({"occurrence": first, "translate": [100, 0, 0]}))["occurrence"].as_u64().unwrap();
    run(&mut s, "occurrence.move", json!({"occurrence": second, "angle": std::f64::consts::FRAC_PI_2, "origin": [100, 0, 0]}));
    run(&mut s, "occurrence.ground", json!({"occurrence": first, "grounded": true}));
    let undo = s.undo.len();
    let before = s.doc.to_json();
    let values = s.doc.assembly.joints[0].values.clone();
    // Drag the non-root side, through the second assembly's rotated frame.
    run(&mut s, "occurrence.drag", json!({"path": [second, b], "at": [100, 0, 0], "point": [110, 0, 0]}));
    near(origin(&s, &[second, a]), [110.0, 0.0, 0.0]);
    near(origin(&s, &[second, b]), [110.0, 0.0, 0.0]);
    near(origin(&s, &[first, a]), [0.0, -10.0, 0.0]);
    near(origin(&s, &[first, b]), [0.0, -10.0, 0.0]);
    let dragged = s.doc.to_json();
    let solution = run(&mut s, "joint.solve", json!({}));
    assert_eq!(solution["conflicts"], json!([]));
    assert_eq!(s.doc.to_json(), dragged, "solving preserves the dragged pose");
    assert_eq!(s.doc.assembly.joints[0].values, values);
    assert_eq!(s.undo.len(), undo + 1);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.to_json(), before);
}

#[test]
fn a_unique_part_can_drag_a_chain_through_a_shared_peer() {
    let mut s = Session::default();
    part(&mut s, "Base", [0.0, 0.0, 0.0]);
    let (_, free) = part(&mut s, "Free", [20.0, 0.0, 0.0]);
    let asm = run(&mut s, "component.create", json!({"name": "Assembly"}));
    let first = asm["occurrence"].as_u64().unwrap();
    let peer = run(&mut s, "component.create", json!({"name": "Peer"}))["occurrence"].as_u64().unwrap();
    run(&mut s, "component.activate", json!({"component": "root"}));
    let second = run(&mut s, "occurrence.copy", json!({"occurrence": first, "translate": [100, 0, 0]}))["occurrence"].as_u64().unwrap();
    let at = |o| json!({"occurrence": o, "point": [0, 0, 0], "z": [0, 0, 1]});
    run(&mut s, "joint.create", json!({"type": "rigid", "a": at(free), "b": at(peer), "flip": true}));
    run(&mut s, "occurrence.drag", json!({"occurrence": free, "at": [0, 0, 0], "point": [0, 15, 0]}));
    near(origin(&s, &[free]), [0.0, 15.0, 0.0]);
    near(origin(&s, &[first, peer]), [0.0, 15.0, 0.0]);
    near(origin(&s, &[second, peer]), [100.0, 15.0, 0.0]);
    let before = s.doc.to_json();
    run(&mut s, "joint.solve", json!({}));
    assert_eq!(s.doc.to_json(), before);
}

/// #11723: Move Occurrence on a part that rigid joints hold to something grounded is put back by
/// the joint solve after it. The move still succeeds, but its reply says it was put back, gives
/// where the part is, and names the joint. A free chain follows the move, as before, and says
/// nothing.
#[test]
fn a_move_its_joints_undo_says_so() {
    let setups: [(&str, fn(&mut Session, u64, u64, u64)); 4] = [
        ("free chain", |_, _, _, _| {}),
        ("plate held to base", |s, base, plate, _| {
            run(s, "joint.as_built", json!({"type": "rigid", "a": base, "b": plate, "name": "Mount"}));
        }),
        ("motor held to base", |s, base, _, motor| {
            run(s, "joint.as_built", json!({"type": "rigid", "a": base, "b": motor, "name": "Mount"}));
        }),
        ("motor grounded", |s, _, _, motor| {
            run(s, "occurrence.ground", json!({"occurrence": motor, "grounded": true}));
        }),
    ];
    for (case, setup) in setups {
        let mut s = Session::default();
        let (_, base) = part(&mut s, "Base", [0.0, 0.0, 0.0]);
        let (_, plate) = part(&mut s, "Plate", [20.0, 0.0, 0.0]);
        let (_, motor) = part(&mut s, "Motor", [20.0, 0.0, 10.0]);
        run(&mut s, "joint.as_built", json!({"type": "rigid", "a": plate, "b": motor, "name": "Bolted"}));
        setup(&mut s, base, plate, motor);
        let (plate_at, motor_at) = (origin(&s, &[plate]), origin(&s, &[motor]));
        let r = run(&mut s, "occurrence.move", json!({"occurrence": plate, "space": "world", "translate": [0, 0, 30]}));
        let reply_at = [r["transform"][3][0].as_f64().unwrap(), r["transform"][3][1].as_f64().unwrap(), r["transform"][3][2].as_f64().unwrap()];
        near(reply_at, origin(&s, &[plate]));
        if case == "free chain" {
            near(origin(&s, &[plate]), [plate_at[0], plate_at[1], plate_at[2] + 30.0]);
            near(origin(&s, &[motor]), [motor_at[0], motor_at[1], motor_at[2] + 30.0]);
            assert!(r.get("reverted").is_none() && r.get("warning").is_none(), "{case}: {r}");
            continue;
        }
        near(origin(&s, &[plate]), plate_at);
        assert_eq!(r["reverted"], json!(true), "{case}: {r}");
        let warning = r["warning"].as_str().unwrap_or_default();
        let holder = if case == "motor grounded" { "Bolted" } else { "Mount" };
        assert!(warning.contains(holder) && warning.contains("joint.reach") && warning.contains("joint.edit"), "{case}: {warning}");
    }
}

/// #11723: a part inside an assembly placed twice has no path of its own. When its joints put it
/// back, the warning still names the joint that holds it.
#[test]
fn a_move_its_joints_undo_names_the_joint_in_a_repeated_assembly() {
    let mut s = Session::default();
    let (_, base) = part(&mut s, "Base", [0.0, 0.0, 0.0]);
    let arm = run(&mut s, "component.create", json!({"name": "Arm"}));
    let (arm_component, arm_at) = (arm["component"].as_u64().unwrap(), arm["occurrence"].as_u64().unwrap());
    run(&mut s, "component.activate", json!({"component": arm_component}));
    let plate = run(&mut s, "component.create", json!({"name": "Plate"}))["occurrence"].as_u64().unwrap();
    run(&mut s, "solid.box", json!({"length": 10, "width": 10, "height": 10, "corner": [20, 0, 0]}));
    run(&mut s, "component.activate", json!({"component": "root"}));
    run(&mut s, "joint.as_built", json!({"type": "rigid", "a": base, "b": plate, "name": "Mount"}));
    run(&mut s, "occurrence.copy", json!({"occurrence": arm_at, "translate": [0, 50, 0]}));
    let r = run(&mut s, "occurrence.move", json!({"occurrence": plate, "translate": [0, 0, 30]}));
    assert_eq!(r["reverted"], json!(true), "{r}");
    let warning = r["warning"].as_str().unwrap_or_default();
    assert!(warning.contains("Mount"), "the warning names the joint: {warning}");
}

#[test]
fn coordinate_frame_delta_matches_origin_and_all_axes_and_refuses_nonrigid_frames() {
    use solidcore_record::Vec3;
    use solidcraft_doc::{IDENTITY, mat_mul, rigid};
    let source = rigid(Vec3::new(7., -4., 12.), Vec3::ZERO, Vec3::new(1., 2., 3.), 0.7);
    let target = rigid(Vec3::new(-8., 2., 19.), Vec3::ZERO, Vec3::X, std::f64::consts::PI);
    let delta = crate::origins::frame_delta(&source, &target).unwrap();
    let aligned = mat_mul(&delta, &source);
    assert!(aligned.iter().flatten().zip(target.iter().flatten()).all(|(a, b)| (a - b).abs() < 1e-10));
    let mut shear = IDENTITY;
    shear[1][0] = 0.2;
    let mut mirror = IDENTITY;
    mirror[0][0] = -1.;
    let mut nan = IDENTITY;
    nan[3][0] = f64::NAN;
    for invalid in [shear, mirror, nan] {
        assert!(crate::origins::frame_delta(&invalid, &target).is_err());
        assert!(crate::origins::frame_delta(&source, &invalid).is_err());
    }
}

#[test]
fn coordinate_frames_include_named_csys_and_planes_in_nested_pending_placements() {
    use solidcore_record::{Plane, Vec3};
    use solidcraft_doc::joints::{JointOrigin, NamedOrigin, Snap};
    use solidcraft_doc::{FeatureKind, PlaneRef, mat_mul, rigid};
    use solidcraft_geom::prelude::*;
    let mut s = Session::default();
    let parent = run(&mut s, "component.create", json!({"name":"Parent","grounded":false}));
    let parent_occ = parent["occurrence"].as_u64().unwrap();
    let child = run(&mut s, "component.create", json!({"name":"Child","grounded":false}));
    let child_occ = child["occurrence"].as_u64().unwrap();
    let child_component = child["component"].as_u64().unwrap();
    let local = rigid(Vec3::new(2., 3., 4.), Vec3::ZERO, Vec3::Y, 0.4);
    s.doc_mut().occurrences.iter_mut().find(|o| o.id == child_occ).unwrap().transform = local;
    let plane = Plane::new(Vec3::new(1., 2., 3.), Vec3::Y, Vec3::Z).unwrap();
    let feature = s.doc_mut().add_feature(FeatureKind::ConstructionPlane { plane: PlaneRef::Custom { plane } }, Some("Mounting frame")).unwrap();
    s.doc_mut().feature_mut(feature).unwrap().component = child_component;
    s.doc_mut().assembly.origins.push(NamedOrigin {
        name: "My CSYS".into(),
        origin: JointOrigin::new(child_occ, Snap::Frame { origin: Vec3::new(4., 5., 6.), z: Vec3::X, x: Some(Vec3::Y) }),
    });
    s.refresh();
    run(&mut s, "occurrence.move", json!({"occurrence":parent_occ,"translate":[10,20,30],"axis":[0,0,1],"angle":"90 deg","capture":false}));
    let world = s.occurrence_placement(&[parent_occ, child_occ]).unwrap().world;
    let expected_origin = mat_mul(&world, &[[0., 1., 0., 0.], [0., 0., 1., 0.], [1., 0., 0., 0.], [4., 5., 6., 1.]]);
    let before = s.doc.to_json();
    let pending = s.pending_moves.clone();
    let revision = s.revision;
    let listed = run(&mut s, "model.coordinate_frames", json!({}));
    let frames = listed["frames"].as_array().unwrap();
    assert!(frames.iter().any(|f| f["key"] == "world"));
    let named = frames.iter().find(|f| f["label"].as_str().unwrap().contains("My CSYS")).unwrap();
    let actual: solidcraft_doc::Mat = serde_json::from_value(named["matrix"].clone()).unwrap();
    assert!(actual.iter().flatten().zip(expected_origin.iter().flatten()).all(|(a, b)| (a - b).abs() < 1e-9));
    let key = named["key"].as_str().unwrap().to_string();
    assert_eq!(s.coordinate_frame(&key).unwrap(), actual);
    let plane_key = format!("plane:{feature}:{parent_occ}/{child_occ}");
    let expected_plane = mat_mul(&world, &[[0., 1., 0., 0.], [0., 0., 1., 0.], [1., 0., 0., 0.], [1., 2., 3., 1.]]);
    let actual = s.coordinate_frame(&plane_key).unwrap();
    assert!(actual.iter().flatten().zip(expected_plane.iter().flatten()).all(|(a, b)| (a - b).abs() < 1e-9));
    assert_eq!(s.doc.to_json(), before);
    assert_eq!(s.pending_moves, pending);
    assert_eq!(s.revision, revision);
    s.doc_mut().feature_mut(feature).unwrap().suppressed = true;
    s.refresh();
    assert!(s.coordinate_frame(&plane_key).is_err());
    s.doc_mut().assembly.origins.clear();
    assert!(s.coordinate_frame(&key).is_err());
    assert!(s.coordinate_frame("missing").is_err());
}

#[test]
fn move_apply_captures_only_the_selected_component_and_undo_restores_its_preview() {
    let mut s = Session::default();
    let first = run(&mut s, "component.create", json!({"name":"First","grounded":false}))["occurrence"].as_u64().unwrap();
    run(&mut s, "component.activate", json!({"component":"root"}));
    let second = run(&mut s, "component.create", json!({"name":"Second","grounded":false}))["occurrence"].as_u64().unwrap();
    run(&mut s, "occurrence.move", json!({"occurrence":first,"translate":[1,2,3],"capture":false}));
    run(&mut s, "occurrence.move", json!({"occurrence":second,"translate":[4,5,6],"capture":false}));
    let before = s.doc.to_json();
    let pending = s.pending_moves.clone();
    let local = pending[&second];
    let rows: Vec<Vec<f64>> = (0..4).map(|r| (0..4).map(|c| local[c][r]).collect()).collect();
    run(&mut s, "occurrence.move", json!({"occurrence":second,"space":"local","to_matrix":rows,"capture":true}));
    assert_eq!(s.pending_moves.get(&first), pending.get(&first));
    assert!(!s.pending_moves.contains_key(&second));
    assert_eq!(s.doc.occurrences.iter().find(|o| o.id == second).unwrap().transform, local);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.to_json(), before);
    assert_eq!(s.pending_moves, pending);
}

#[test]
fn geometry_handle_snaps_follow_nested_pending_placements() {
    use crate::geometry_snap::{GeometryKind, GeometryPick, SnapMode};
    use solidcore_record::brep::Curve;
    let mut s = Session::default();
    let parent = run(&mut s, "component.create", json!({"name":"Snap Parent","grounded":false}))["occurrence"].as_u64().unwrap();
    let child = run(&mut s, "component.create", json!({"name":"Snap Child","grounded":false}))["occurrence"].as_u64().unwrap();
    run(&mut s, "solid.box", json!({"length":10,"width":20,"height":30,"corner":[0,0,0]}));
    run(&mut s, "occurrence.move", json!({"occurrence":parent,"translate":[100,200,300],"axis":[0,0,1],"angle":"90 deg","capture":false}));
    run(&mut s, "occurrence.move", json!({"occurrence":child,"translate":[4,5,6],"capture":false}));
    let world = s.world_state().unwrap();
    let body = &world.bodies[0];
    let record = body.body.record();
    let (edge, e) = record.edges.iter().enumerate().find(|(_, e)| matches!(e.curve, Curve::Line { .. })).unwrap();
    let a = record.vertices[e.vertices[0] as usize].point;
    let b = record.vertices[e.vertices[1] as usize].point;
    let expected = [(a[0] + b[0]) / 2., (a[1] + b[1]) / 2., (a[2] + b[2]) / 2.];
    let pick = GeometryPick { body: body.name.clone(), kind: GeometryKind::Edge, index: Some(edge), point: expected };
    let before = s.doc.to_json();
    let pending = s.pending_moves.clone();
    let revision = s.revision;
    let mid = s.geometry_snap(&pick, SnapMode::Midpoint).unwrap();
    near(mid.point, expected);
    near(s.geometry_snap(&pick, SnapMode::Point).unwrap().point, expected);
    let dir = mid.direction.unwrap();
    assert!((dir.iter().map(|v| v * v).sum::<f64>() - 1.).abs() < 1e-10);
    let vertex = GeometryPick { kind: GeometryKind::Vertex, index: None, point: a, ..pick.clone() };
    near(s.geometry_snap(&vertex, SnapMode::Point).unwrap().point, a);
    assert!(s.geometry_snap(&pick, SnapMode::Center).is_err());
    let invalid = GeometryPick { index: Some(usize::MAX), ..pick };
    assert!(s.geometry_snap(&invalid, SnapMode::Point).is_err());
    assert_eq!(s.doc.to_json(), before);
    assert_eq!(s.pending_moves, pending);
    assert_eq!(s.revision, revision);
    let min = |axis: usize| record.vertices.iter().map(|v| v.point[axis]).fold(f64::INFINITY, f64::min);
    near([min(0), min(1), min(2)], [75., 204., 306.]);
}
#[test]
fn geometry_handle_face_and_circle_samples_keep_analytic_normals() {
    use crate::geometry_snap::{GeometryKind, GeometryPick, SnapMode};
    use solidcore_record::brep::{Curve, Surface};
    let mut s = Session::default();
    run(&mut s, "solid.cylinder", json!({"diameter":10,"height":20,"base":[0,0,0]}));
    let world = s.world_state().unwrap();
    let body = &world.bodies[0];
    let record = body.body.record();
    let (edge, frame) =
        record.edges.iter().enumerate().find_map(|(i, e)| if let Curve::Circle { frame, .. } = &e.curve { Some((i, frame)) } else { None }).unwrap();
    let pick = GeometryPick { body: body.name.clone(), kind: GeometryKind::Edge, index: Some(edge), point: [5., 0., 0.] };
    near(s.geometry_snap(&pick, SnapMode::Center).unwrap().point, frame.origin);
    assert!(s.geometry_straight_edge(&pick).is_err());
    let face = record.faces.iter().position(|f| matches!(f.surface, Surface::Cylinder { .. })).unwrap();
    let pick = GeometryPick { kind: GeometryKind::Face, index: Some(face), point: [6., 0., 10.], ..pick };
    let snapped = s.geometry_snap(&pick, SnapMode::Point).unwrap();
    near(snapped.point, [5., 0., 10.]);
    near(snapped.direction.unwrap(), [1., 0., 0.]);
    let queried = run(&mut s, "model.geometry_snap", json!({"pick":pick,"mode":"point"}));
    near(serde_json::from_value(queried["point"].clone()).unwrap(), [5., 0., 10.]);
}
#[test]
fn geometry_handle_virtual_intersection_and_axis_flip_are_true_3d() {
    use crate::geometry_snap::align_axis;
    use solidcraft_doc::IDENTITY;
    let cross = solidcraft_kernel::geometry_snap::line_intersection([[0., 0., 0.], [1., 0., 0.]], [[3., 2., 0.], [3., 3., 0.]], 1e-6).unwrap();
    near(cross.point, [3., 0., 0.]);
    assert!(solidcraft_kernel::geometry_snap::line_intersection([[0., 0., 0.], [1., 0., 0.]], [[3., 2., 1.], [3., 3., 1.]], 1e-6).is_err());
    assert!(solidcraft_kernel::geometry_snap::line_intersection([[0., 0., 0.], [1., 0., 0.]], [[0., 1., 0.], [1., 1., 0.]], 1e-6).is_err());
    let mut frame = IDENTITY;
    frame[3] = [10., 20., 30., 1.];
    for axis in 0..3 {
        let m = align_axis(&frame, axis, [1., 2., 3.], false).unwrap();
        assert!(solidcraft_doc::is_placement(&m));
        assert_eq!(m[3], frame[3]);
        let n = 14f64.sqrt();
        near([m[axis][0], m[axis][1], m[axis][2]], [1. / n, 2. / n, 3. / n]);
    }
    let flipped = align_axis(&frame, 0, [1., 0., 0.], true).unwrap();
    assert!(solidcraft_doc::is_placement(&flipped));
    near([flipped[0][0], flipped[0][1], flipped[0][2]], [-1., 0., 0.]);
    assert!(align_axis(&frame, 3, [1., 0., 0.], false).is_err());
    assert!(align_axis(&frame, 0, [0., 0., 0.], false).is_err());
}

#[test]
fn geometry_handle_face_centers_use_bounded_axis_and_follow_component_moves() {
    use crate::geometry_snap::{GeometryKind, GeometryPick, SnapMode};
    use solidcore_record::brep::Surface;
    for (command, params) in [
        ("solid.cylinder", json!({"diameter":10,"height":20,"base":[3,4,5]})),
        ("solid.cone", json!({"radius":10,"top_radius":4,"height":20,"base":[3,4,5]})),
        ("solid.cone", json!({"radius":10,"height":20,"base":[3,4,5]})),
    ] {
        let mut s = Session::default();
        let occurrence = run(&mut s, "component.create", json!({"name":"Axial part","grounded":false}))["occurrence"].as_u64().unwrap();
        run(&mut s, command, params);
        // Z becomes X; the face centre must follow the pending placement.
        run(&mut s, "occurrence.move", json!({"occurrence":occurrence,"translate":[100,200,300],"axis":[0,1,0],"angle":"90 deg","capture":false}));
        let world = s.world_state().unwrap();
        let body = &world.bodies[0];
        let index = body.body.record().faces.iter().position(|f| matches!(f.surface, Surface::Cylinder { .. } | Surface::Cone { .. })).unwrap();
        let pick = GeometryPick { body:body.name.clone(), kind:GeometryKind::Face, index:Some(index), point:[0.,0.,0.] };
        let before = s.doc.to_json();
        let pending = s.pending_moves.clone();
        let snap = s.geometry_snap(&pick, SnapMode::Center).unwrap();
        near(snap.point, [115.,204.,297.]);
        // Cone carriers may reverse their canonical axis; either sign is coaxial.
        let direction = snap.direction.unwrap();
        near([direction[0].abs(),direction[1].abs(),direction[2].abs()], [1.,0.,0.]);
        let other_hint = GeometryPick { point:[500.,-300.,14.], ..pick.clone() };
        near(s.geometry_snap(&other_hint, SnapMode::Center).unwrap().point, snap.point);
        let queried = run(&mut s, "model.geometry_snap", json!({"pick":pick,"mode":"center"}));
        near(serde_json::from_value(queried["point"].clone()).unwrap(), snap.point);
        assert_eq!(s.doc.to_json(), before);
        assert_eq!(s.pending_moves, pending);
    }
}
