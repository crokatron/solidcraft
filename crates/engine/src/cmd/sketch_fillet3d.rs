//! A circular fillet in the plane spanned by two drawn sketch lines.
use super::{edit, inverse_owner_frame, needs_3d, reply, sketch_store};
use crate::{
    Result, Session,
    params::{bad, req_expr},
    schema::sketch_3d::Curves3dResult,
};
use serde_json::Value;
use solidcore_record::{Plane, Vec3};
use solidcraft_doc::{Mat, apply_point, mat_inverse};
use solidcraft_geom::prelude::*;
use solidcraft_sketch::{CurveKind, Sketch, WireSpace};

const CMD: &str = "sketch.fillet3d";
enum Source {
    Wire(usize, WireSpace),
    Planar(usize, [usize; 2]),
}
struct Line {
    source: Source,
    ends: [Vec3; 2],
}

fn line(sk: &Sketch, id: &str, plane: &Plane, inverse: &Mat) -> Result<Line> {
    if let Some((i, w)) = sk.wire_index(id).and_then(|i| sk.wires.get(i).map(|w| (i, w))) {
        let [a, b] = w.fit.as_slice() else { return Err(bad(CMD, format!("{id} is not a drawn straight line"))) };
        if w.link.is_some() || w.arc.is_some() || !w.exact.is_empty() {
            return Err(bad(CMD, "linked or curved geometry cannot be trimmed"));
        }
        let ends = [*a, *b].map(|p| if w.space == WireSpace::World { apply_point(inverse, p) } else { p });
        return Ok(Line { source: Source::Wire(i, w.space), ends });
    }
    let i = sk.curve_index(id).ok_or_else(|| bad(CMD, format!("unknown sketch line {id}")))?;
    let c = &sk.curves[i];
    let CurveKind::Line { a, b } = c.kind else { return Err(bad(CMD, "pick two straight lines")) };
    if c.link.is_some() || c.fixed || c.infinite || sk.is_ellipse_axis(i) || sk.offset_distance(id).is_some() {
        return Err(bad(CMD, "pick finite editable lines; a linked, fixed or generated line cannot be trimmed"));
    }
    let point = |j| sk.point(j).map(|q| plane.to_world(q)).ok_or_else(|| bad(CMD, "missing line endpoint"));
    Ok(Line { source: Source::Planar(i, [a, b]), ends: [point(a)?, point(b)?] })
}

struct Round {
    cut: [usize; 2],
    arc: [Vec3; 3],
}
fn construct(a: [Vec3; 2], b: [Vec3; 2], radius: f64) -> Result<Round> {
    let unit = |p: [Vec3; 2]| (p[1] - p[0]).normalized().ok_or_else(|| bad(CMD, "zero length line"));
    let (x, y) = (unit(a)?, unit(b)?);
    let n = x.cross(y);
    let n2 = n.len2();
    if n2 < 1e-16 {
        return Err(bad(CMD, "parallel or collinear lines have no fillet corner"));
    }
    let between = b[0] - a[0];
    let pa = a[0] + x * (between.cross(y).dot(n) / n2);
    let pb = b[0] + y * (between.cross(x).dot(n) / n2);
    let tol = 1e-8 + 1e-10 * a[0].dist(a[1]).max(b[0].dist(b[1]));
    if pa.dist(pb) > tol {
        return Err(bad(CMD, "skew lines do not meet in a common plane"));
    }
    let corner = pa.lerp(pb, 0.5);
    // Keep the end farther from the virtual intersection. A tied interior intersection
    // has no unique corner; picking a joined pair disambiguates it.
    let near = |p: [Vec3; 2]| -> Result<usize> {
        let d = [p[0].dist(corner), p[1].dist(corner)];
        if (d[0] - d[1]).abs() <= tol {
            return Err(bad(CMD, "the crossing has two equally near ends; split the lines at the intended corner first"));
        }
        Ok(usize::from(d[1] < d[0]))
    };
    let cut = [near(a)?, near(b)?];
    let far = [a[1 - cut[0]], b[1 - cut[1]]];
    let u = (far[0] - corner).normalized().ok_or_else(|| bad(CMD, "degenerate corner"))?;
    let v = (far[1] - corner).normalized().ok_or_else(|| bad(CMD, "degenerate corner"))?;
    let half = 0.5 * u.cross(v).len().atan2(u.dot(v));
    let bis = (u + v).normalized().ok_or_else(|| bad(CMD, "straight corner"))?;
    let setback = radius / half.tan();
    for (j, seg) in [a, b].iter().enumerate() {
        let d = [u, v][j];
        let least = (seg[cut[j]] - corner).dot(d);
        if setback < least - tol || setback >= far[j].dist(corner) - tol {
            return Err(bad(CMD, "radius does not fit the lengths of both line segments"));
        }
    }
    let mid = corner + bis * (radius * (half.cos() * half.cos()) / (half.sin() * (1. + half.sin())));
    Ok(Round { cut, arc: [corner + u * setback, mid, corner + v * setback] })
}
fn trim(sk: &mut Sketch, line: &Line, cut: usize, touch: Vec3, plane: &Plane, world: &Mat) -> Result<()> {
    match line.source {
        Source::Wire(i, space) => {
            let p = if space == WireSpace::World { apply_point(world, touch) } else { touch };
            let w = &mut sk.wires[i];
            w.fit[cut] = p;
            w.pts = w.fit.clone();
        }
        Source::Planar(curve, ends) => {
            let q = plane.to_local(touch);
            let old = ends[cut];
            // A shared corner also belongs to other geometry. Replace just this line's
            // endpoint and leave that geometry and its constraints in place.
            if sk.point_locked(old)
                || sk.curves.iter().enumerate().any(|(i, c)| i != curve && c.kind.uses(old))
                || sk.constraints.iter().any(|c| c.kind.refers_to_point(old))
            {
                let endpoint = sk.add_point(q, None)?;
                let CurveKind::Line { a, b } = &mut sk.curves[curve].kind else { return Err(bad(CMD, "expected a line")) };
                if cut == 0 { *a = endpoint } else { *b = endpoint }
            } else {
                let pt = sk.points.get_mut(old).ok_or_else(|| bad(CMD, "missing planar endpoint"))?;
                pt.pos = q;
            }
        }
    }
    Ok(())
}
pub(super) fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let args: crate::schema::sketch_3d::Fillet3dParams = serde_json::from_value(p.clone()).map_err(|e| bad(CMD, e.to_string()))?;
    if args.curves[0] == args.curves[1] {
        return Err(bad(CMD, "pick two different lines"));
    }
    let id = sketch_store::target(s, p, CMD)?;
    let plane = s.model.state().sketch(id).ok_or_else(|| bad(CMD, "sketch has no plane"))?.plane;
    let inverse = inverse_owner_frame(s, p, CMD)?;
    let world = mat_inverse(&inverse).ok_or_else(|| bad(CMD, "singular sketch placement"))?;
    let expr = req_expr(CMD, p, "radius")?;
    let ((arc, expected), info) = edit(s, p, CMD, false, |sk, doc| {
        needs_3d(sk, CMD)?;
        let r = doc.eval(&expr, solidcraft_doc::expr::Kind::Length).map_err(|e| bad(CMD, e.to_string()))?;
        if !(1e-9..1e8).contains(&r) {
            return Err(bad(CMD, "radius must be positive and within sketch bounds"));
        }
        let a = line(sk, &args.curves[0], &plane, &inverse)?;
        let b = line(sk, &args.curves[1], &plane, &inverse)?;
        let round = construct(a.ends, b.ends, r)?;
        // Rounding replaces the coincidence at this corner with two tangent arc ends.
        // Other relations stay in place and must still be satisfiable after the trim.
        if let (Source::Wire(ai, _), Source::Wire(bi, _)) = (&a.source, &b.source) {
            let pa = solidcraft_sketch::wire_constraints::WirePoint { wire: sk.wires[*ai].id.clone(), index: round.cut[0] };
            let pb = solidcraft_sketch::wire_constraints::WirePoint { wire: sk.wires[*bi].id.clone(), index: round.cut[1] };
            sk.wire_constraints.retain(|c| {
                !(c.kind == solidcraft_sketch::wire_constraints::Relation::Coincident && ((c.a == pa && c.b == pb) || (c.a == pb && c.b == pa)))
            });
        }
        trim(sk, &a, round.cut[0], round.arc[0], &plane, &world)?;
        trim(sk, &b, round.cut[1], round.arc[2], &plane, &world)?;
        let expected = [(round.cut[0], round.arc[0], a.ends[1 - round.cut[0]]), (round.cut[1], round.arc[2], b.ends[1 - round.cut[1]])];
        // Keep legacy world-space wires world-fixed when both operands use that space.
        let w = matches!((&a.source, &b.source), (Source::Wire(_, WireSpace::World), Source::Wire(_, WireSpace::World)));
        let points = if w { round.arc.map(|p| apply_point(&world, p)) } else { round.arc };
        let arc = sk.add_drawn_arc_in(&points, if w { WireSpace::World } else { WireSpace::Component })?;
        for (operand, end) in [(&a, 0), (&b, 2)] {
            if let Source::Wire(index, space) = operand.source {
                if space != if w { WireSpace::World } else { WireSpace::Component } {
                    continue;
                }
                let reference = sk.wires[index].id.clone();
                sk.add_wire_constraint(solidcraft_sketch::wire_constraints::Relation::Tangent, &reference, &format!("{arc}:{end}"))?;
            }
        }
        Ok((arc, expected))
    })?;
    // A planar operand may have a driving length or other solver constraints. Never leave
    // a nominal fillet whose line shifted away from the requested tangency during solve.
    let sk = sketch_store::read(&s.doc, id)?;
    for (name, (cut, touch, far)) in args.curves.iter().zip(expected) {
        let l = line(sk, name, &plane, &inverse)?;
        let tol = 1e-7 * (1. + touch.dist(far));
        if l.ends[cut].dist(touch) > tol || l.ends[1 - cut].dist(far) > tol {
            return Err(bad(CMD, "a sketch constraint prevents trimming this line tangentially"));
        }
    }
    reply(CMD, Curves3dResult { curves: vec![arc], sketch: info, param_warnings: Vec::new() })
}
