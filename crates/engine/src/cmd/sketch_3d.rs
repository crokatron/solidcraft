//! 3D sketch drawing: with the sketch's 3D Sketch option on (Sketch Palette), lines, arcs,
//! splines and points may leave the sketch plane. They are kept as drawn 3D curves (wires with
//! fit points, exact arcs keeping their analytic form): drawn and snapped to, movable point by
//! point, and usable as sweep and pipe paths.

use serde_json::{Value, json};
use solidcore_record::Vec3;
use solidcraft_sketch::WireSpace;

use super::sketch::edit;
use super::sketch_store;
use super::{CommandSpec, in_sketch};
use crate::params::{bad, req_vec3, vec3};
use crate::schema::sketch_3d::{Curves3dResult, Line3dParams, Move3dParams, Move3dResult, Point3dParams, Point3dResult, Spline3dParams};
use crate::schema::{TypedContract, output_schema_for};
use crate::{Result, Session};

#[path = "sketch_fillet3d.rs"]
mod fillet;

const LINE3D_CONTRACT: TypedContract<Line3dParams> = TypedContract::new();
const SPLINE3D_CONTRACT: TypedContract<Spline3dParams> = TypedContract::new();
const POINT3D_CONTRACT: TypedContract<Point3dParams> = TypedContract::new();
const MOVE3D_CONTRACT: TypedContract<Move3dParams> = TypedContract::new();

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("sketch.dimension3d", "3D Dimension", crate::checked!("sketch.dimension3d", dimension3d))
        .icon("dimension").enabled(in_sketch)
        .contract(&TypedContract::<crate::schema::sketch_3d::Dimension3dParams>::new())
        .output(output_schema_for::<crate::schema::sketch_3d::Dimension3dResult>)
        .params("kind: distance|x|y|z|radius|angle|fix, a: editable 3D curve or fit-point ID, b?: second operand, value?: mm or degrees for angle; signed axis dimensions use B minus A in the stored curve frame, not the camera. A bare line measures its endpoints. Fix on a bare curve holds every defining point."),
    CommandSpec::new("sketch.dimension3d.edit", "Edit 3D Dimension", crate::checked!("sketch.dimension3d.edit", edit_dimension3d))
        .icon("dimension").enabled(in_sketch)
        .contract(&TypedContract::<crate::schema::sketch_3d::EditDimension3dParams>::new())
        .output(output_schema_for::<crate::schema::sketch_3d::Dimension3dResult>)
        .params("id: saved 3D dimension ID, value: new mm or degrees for angle; conflicting edits leave the design unchanged"),
    CommandSpec::new("sketch.freedom3d", "3D Sketch Freedom", crate::checked!("sketch.freedom3d", freedom3d))
        .icon("dimension").noundo()
        .contract(&TypedContract::<crate::schema::sketch_3d::Freedom3dParams>::new())
        .output(output_schema_for::<crate::schema::sketch_3d::Freedom3dResult>)
        .params("sketch?: target sketch; report instantaneous spatial DOF and each point's admissible movement directions, including rigid motion. No document changes."),
    CommandSpec::new("sketch.fillet3d", "3D Sketch Fillet", crate::checked!("sketch.fillet3d",fillet::run))
        .icon("sketch_fillet").enabled(in_sketch_3d)
        .contract(&TypedContract::<crate::schema::sketch_3d::Fillet3dParams>::new())
        .output(output_schema_for::<Curves3dResult>)
        .params("curves: two editable straight line IDs in the active 3D sketch, radius: positive length in mm or expression; trim each line to meet an exact tangent circular arc"),
    CommandSpec::new("sketch.line3d", "3D Line", crate::checked!("sketch.line3d", line3d))
        .icon("line")
        .enabled(in_sketch_3d)
        .contract(&LINE3D_CONTRACT)
        .output(output_schema_for::<Curves3dResult>)
        .params(concat!(
            "points: [[x,y,z]…] world points (2 or more): one 3D line per segment, joined end to end (needs 3D Sketch on)",
            created_doc!()
        )),
    CommandSpec::new("sketch.spline3d", "3D Spline", crate::checked!("sketch.spline3d", spline3d))
        .icon("spline")
        .enabled(in_sketch_3d)
        .contract(&SPLINE3D_CONTRACT)
        .output(output_schema_for::<Curves3dResult>)
        .params(concat!("points: [[x,y,z]…] world fit points (2 or more): a smooth 3D curve through them (needs 3D Sketch on)", created_doc!())),
    CommandSpec::new("sketch.arc3d", "3D Arc", arc3d)
        .icon("arc3")
        .enabled(in_sketch_3d)
        .params(concat!(
            "start, through, end: [x,y,z] world points: an exact circular arc through them, from start through through to end (needs 3D Sketch on). Stored exactly (centre, radius, angles), so path consumers can treat it as a true arc",
            created_doc!()
        )),
    CommandSpec::new("sketch.point3d", "3D Point", crate::checked!("sketch.point3d", point3d))
        .icon("point")
        .enabled(in_sketch_3d)
        .contract(&POINT3D_CONTRACT)
        .output(output_schema_for::<Point3dResult>)
        .params(concat!("point: [x,y,z] world point (needs 3D Sketch on)", created_doc!())),
    CommandSpec::new("sketch.move3d", "Move 3D Point", crate::checked!("sketch.move3d", move3d))
        .icon("sketch_move")
        .enabled(in_sketch)
        .contract(&MOVE3D_CONTRACT)
        .output(output_schema_for::<Move3dResult>)
        .params("wire: drawn 3D curve or point id, index?: fit point (default 0), to: [x,y,z] | by: [dx,dy,dz]; points joined to it move too"),
];

/// A command's reply as JSON: built from the type its output schema describes, so the two can't
/// drift apart.
fn reply(cmd: &str, reply: impl serde::Serialize) -> Result<Value> {
    serde_json::to_value(reply).map_err(|e| bad(cmd, e.to_string()))
}

fn points(cmd: &str, p: &Value, min: usize) -> Result<Vec<Vec3>> {
    let a = p.get("points").and_then(Value::as_array).ok_or_else(|| bad(cmd, "`points` must list [x,y,z] points"))?;
    if a.len() < min || a.len() > solidcraft_sketch::MAX_FIT_POINTS {
        return Err(bad(cmd, format!("`points` needs {min} to {} points", solidcraft_sketch::MAX_FIT_POINTS)));
    }
    a.iter().map(|v| vec3(v).ok_or_else(|| bad(cmd, "a point must be [x, y, z]"))).collect()
}

fn needs_3d(sk: &solidcraft_sketch::Sketch, cmd: &str) -> Result<()> {
    if sk.view.three_d { Ok(()) } else { Err(bad(cmd, "turn on 3D Sketch (sketch.options sketch_3d: true) to draw off the sketch plane")) }
}

/// The 3D Line, Spline, Arc and Point buttons start interactive tools, so they are only offered
/// while a 3D sketch is being edited; otherwise the tools could only fail on their first click.
fn in_sketch_3d(s: &Session) -> std::result::Result<(), String> {
    let id = sketch_store::active(s).ok_or("no sketch is being edited (Create Sketch or Edit Sketch first)")?;
    match sketch_store::read(&s.doc, id) {
        Ok(sk) if sk.view.three_d => Ok(()),
        Ok(_) => Err("turn on 3D Sketch (sketch.options sketch_3d: true) to draw 3D curves".into()),
        Err(_) => Err("no sketch is being edited (Create Sketch or Edit Sketch first)".into()),
    }
}

/// World command inputs belong to the target sketch's owner, which may differ from the
/// active component when a script supplies an explicit sketch.
fn inverse_owner_frame(s: &Session, p: &Value, cmd: &str) -> Result<solidcraft_doc::Mat> {
    let id = sketch_store::target(s, p, cmd)?;
    let owner = s.doc.feature(id).map(|f| f.component).ok_or_else(|| bad(cmd, "target sketch is missing"))?;
    if owner != 0 && owner != s.active_component {
        if s.doc.occurrences.len() > 100_000 {
            return Err(bad(cmd, "sketch owner placement expansion exceeds supported bounds"));
        }
        let mut children: std::collections::BTreeMap<u64, Vec<&solidcraft_doc::Occurrence>> = std::collections::BTreeMap::new();
        for occurrence in &s.doc.occurrences {
            children.entry(occurrence.parent).or_default().push(occurrence);
        }
        let mut stack = vec![(0, 0usize)];
        let mut count = 0;
        let mut visited = 0;
        while let Some((component, depth)) = stack.pop() {
            for o in children.get(&component).into_iter().flatten() {
                visited += 1;
                if depth >= 64 || visited > 100_000 {
                    return Err(bad(cmd, "sketch owner placement expansion exceeds supported bounds"));
                }
                if o.component == owner {
                    count += 1;
                    if count > 1 {
                        return Err(bad(cmd, "activate the desired sketch owner occurrence before editing world 3D points"));
                    }
                }
                stack.push((o.component, depth + 1));
            }
        }
    }
    let frame = if owner == s.active_component { s.active_component_frame() } else { s.placed_document().component_transform(owner) };
    solidcraft_doc::mat_inverse(&frame).ok_or_else(|| bad(cmd, "the sketch owner placement is not invertible"))
}

fn line3d(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "sketch.line3d";
    let inverse = inverse_owner_frame(s, p, cmd)?;
    let pts: Vec<Vec3> = points(cmd, p, 2)?.into_iter().map(|at| solidcraft_doc::apply_point(&inverse, at)).collect();
    let (ids, info) = edit(s, p, cmd, false, |sk, _| {
        needs_3d(sk, cmd)?;
        pts.windows(2).map(|w| sk.add_drawn_wire_in(w, WireSpace::Component).map_err(Into::into)).collect::<Result<Vec<String>>>()
    })?;
    reply(cmd, Curves3dResult { curves: ids, sketch: info, param_warnings: Vec::new() })
}

fn spline3d(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "sketch.spline3d";
    let inverse = inverse_owner_frame(s, p, cmd)?;
    let pts: Vec<Vec3> = points(cmd, p, 2)?.into_iter().map(|at| solidcraft_doc::apply_point(&inverse, at)).collect();
    let (id, info) = edit(s, p, cmd, false, |sk, _| {
        needs_3d(sk, cmd)?;
        Ok(sk.add_drawn_wire_in(&pts, WireSpace::Component)?)
    })?;
    reply(cmd, Curves3dResult { curves: vec![id], sketch: info, param_warnings: Vec::new() })
}

fn arc3d(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "sketch.arc3d";
    let inverse = inverse_owner_frame(s, p, cmd)?;
    let pts: Vec<Vec3> = ["start", "through", "end"]
        .iter()
        .map(|k| req_vec3(cmd, p, k))
        .collect::<Result<Vec<Vec3>>>()?
        .into_iter()
        .map(|at| solidcraft_doc::apply_point(&inverse, at))
        .collect();
    let (id, info) = edit(s, p, cmd, false, |sk, _| {
        needs_3d(sk, cmd)?;
        Ok(sk.add_drawn_arc_in(&pts, WireSpace::Component)?)
    })?;
    Ok(json!({"curves": [id], "sketch": info}))
}

fn point3d(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "sketch.point3d";
    let inverse = inverse_owner_frame(s, p, cmd)?;
    let at = solidcraft_doc::apply_point(&inverse, req_vec3(cmd, p, "point")?);
    let (id, info) = edit(s, p, cmd, false, |sk, _| {
        needs_3d(sk, cmd)?;
        Ok(sk.add_drawn_wire_in(&[at], WireSpace::Component)?)
    })?;
    reply(cmd, Point3dResult { point: id, sketch: info, param_warnings: Vec::new() })
}

fn move3d(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "sketch.move3d";
    let wire = p.get("wire").and_then(Value::as_str).ok_or_else(|| bad(cmd, "`wire` must be a 3D curve or point id"))?.to_string();
    let index = match p.get("index") {
        None => 0,
        Some(v) => v
            .as_u64()
            .and_then(|v| usize::try_from(v).ok())
            .filter(|v| *v < solidcraft_sketch::MAX_FIT_POINTS)
            .ok_or_else(|| bad(cmd, "`index` must be a fit point index below 500"))?,
    };
    let parse = |name| p.get(name).map(|v| vec3(v).ok_or_else(|| bad(cmd, format!("`{name}` must be [x,y,z]")))).transpose();
    let to = parse("to")?;
    let by = parse("by")?;
    let inverse = inverse_owner_frame(s, p, cmd)?;
    let (n, info) = edit(s, p, cmd, false, |sk, _| {
        let drawn = sk
            .wire_index(&wire)
            .and_then(|w| sk.wires.get(w))
            .filter(|w| !w.fit.is_empty() && w.link.is_none())
            .ok_or_else(|| bad(cmd, format!("`{wire}` is not an editable drawn 3D curve")))?;
        let from = drawn.fit.get(index).copied().ok_or_else(|| bad(cmd, format!("`{wire}` has no drawn point {index}")))?;
        let to = match (to, by) {
            (Some(t), _) => {
                if drawn.space == WireSpace::Component {
                    solidcraft_doc::apply_point(&inverse, t)
                } else {
                    t
                }
            }
            (None, Some(d)) => from + if drawn.space == WireSpace::Component { solidcraft_doc::apply_vector(&inverse, d) } else { d },
            _ => return Err(bad(cmd, "give `to` or `by`")),
        };
        Ok(sk.move_drawn_point(&wire, index, to)?)
    })?;
    reply(cmd, Move3dResult { moved: n, sketch: info, param_warnings: Vec::new() })
}

#[cfg(test)]
#[path = "sketch_3d_frame_tests.rs"]
mod frame_tests;

/// Route existing Coincident/Tangent buttons to actual spatial relations before any planar
/// projection. One spatial operand paired with a planar entity is refused explicitly.
pub(super) fn constrain_3d(s: &mut Session, p: &Value, kind: solidcraft_sketch::wire_constraints::Relation) -> Result<Option<Value>> {
    let cmd =
        if kind == solidcraft_sketch::wire_constraints::Relation::Coincident { "sketch.constraint.coincident" } else { "sketch.constraint.tangent" };
    let id = sketch_store::target(s, p, cmd)?;
    let prefix = format!("{id}:");
    let name = |key| p.get(key).and_then(Value::as_str).map(|s| s.strip_prefix(&prefix).unwrap_or(s).to_string());
    let a = name("a");
    let b = name("b");
    let sk = sketch_store::read(&s.doc, id)?;
    if ![a.as_deref(), b.as_deref()].into_iter().flatten().any(|id| sk.wire_operand(id).is_some()) {
        return Ok(None);
    }
    let (Some(a), Some(b)) = (a, b) else {
        return Err(bad(cmd, "3D constraints need two editable spatial curve or endpoint ids"));
    };
    let (constraint, info) = edit(s, p, cmd, true, |sk, _| Ok(sk.add_wire_constraint(kind, &a, &b)?))?;
    Ok(Some(json!({"constraint":constraint,"sketch":info})))
}

fn dimensional_kind(name: &str) -> Option<solidcraft_sketch::wire_constraints::Relation> {
    use solidcraft_sketch::wire_constraints::Relation::*;
    Some(match name {
        "distance" => Distance,
        "x" => DeltaX,
        "y" => DeltaY,
        "z" => DeltaZ,
        "radius" => Radius,
        "angle" => Angle,
        "fix" => Fix,
        _ => return None,
    })
}
fn dimension3d(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "sketch.dimension3d";
    let kind = p
        .get("kind")
        .and_then(Value::as_str)
        .and_then(dimensional_kind)
        .ok_or_else(|| bad(cmd, "kind must be distance, x, y, z, radius, angle or fix"))?;
    let target = sketch_store::target(s, p, cmd)?;
    let prefix = format!("{target}:");
    let a = p.get("a").and_then(Value::as_str).ok_or_else(|| bad(cmd, "a must name a spatial curve or fit point"))?;
    let a = a.strip_prefix(&prefix).unwrap_or(a);
    let b = p.get("b").map(|v| v.as_str().ok_or_else(|| bad(cmd, "b must name a spatial curve or fit point"))).transpose()?;
    let b = b.map(|b| b.strip_prefix(&prefix).unwrap_or(b));
    use solidcraft_sketch::wire_constraints::Relation;
    let value = if kind == Relation::Fix {
        0.
    } else {
        p.get("value").and_then(Value::as_f64).filter(|v| v.is_finite()).ok_or_else(|| bad(cmd, "value must be a finite number"))?
    };
    let value = if kind == Relation::Angle { value.to_radians() } else { value };
    let (constraints, info) = edit(s, p, cmd, true, |sk, _| {
        Ok(if kind == Relation::Fix { sk.fix_wire_geometry(a)? } else { vec![sk.add_wire_dimension(kind, a, b, value)?] })
    })?;
    reply(cmd, crate::schema::sketch_3d::Dimension3dResult { constraints, sketch: info })
}
fn edit_dimension3d(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "sketch.dimension3d.edit";
    let id = p.get("id").and_then(Value::as_str).ok_or_else(|| bad(cmd, "id must name a 3D dimension"))?;
    let value = p.get("value").and_then(Value::as_f64).filter(|v| v.is_finite()).ok_or_else(|| bad(cmd, "value must be finite"))?;
    let (_, info) = edit(s, p, cmd, true, |sk, _| {
        let c = sk.wire_constraints.iter().find(|c| c.id == id).ok_or_else(|| bad(cmd, "unknown 3D dimension"))?;
        let value = if c.kind == solidcraft_sketch::wire_constraints::Relation::Angle { value.to_radians() } else { value };
        Ok(sk.set_wire_dimension(id, value)?)
    })?;
    reply(cmd, crate::schema::sketch_3d::Dimension3dResult { constraints: vec![id.into()], sketch: info })
}
fn freedom3d(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "sketch.freedom3d";
    let id = sketch_store::target(s, p, cmd)?;
    let freedom = sketch_store::read(&s.doc, id)?.wire_freedom();
    let result: crate::schema::sketch_3d::Freedom3dResult =
        serde_json::from_value(serde_json::to_value(freedom).map_err(|e| bad(cmd, e.to_string()))?).map_err(|e| bad(cmd, e.to_string()))?;
    reply(cmd, result)
}
