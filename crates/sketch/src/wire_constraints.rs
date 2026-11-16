//! Persistent constraints for editable spatial fit curves. Each connected set is solved
//! independently by bounded damped least squares; unsupported or conflicting edits are atomic.
use crate::model::Result;
use crate::{Sketch, SketchError, Wire};
use serde::{Deserialize, Serialize};
use solidcore_record::Vec3;
use solidcraft_geom::prelude::*;
use std::collections::BTreeSet;

#[path = "wire_freedom.rs"]
mod freedom;
pub use freedom::{SpatialFreedom, SpatialPointFreedom};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    Coincident,
    Tangent,
    Distance,
    DeltaX,
    DeltaY,
    DeltaZ,
    Radius,
    Angle,
    Fix,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WirePoint {
    pub wire: String,
    pub index: usize,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WireConstraint {
    pub id: String,
    pub kind: Relation,
    pub a: WirePoint,
    pub b: WirePoint,
    #[serde(default)]
    pub same_direction: bool,
    /// Driving millimetres, or radians for Angle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    /// A fixed fit point in the wire coordinate frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<Vec3>,
}
const LIMIT: usize = 256;
const TOL: f64 = 1e-7;
fn invalid(message: &str) -> SketchError {
    SketchError::Invalid(message.into())
}

impl Sketch {
    /// Wire ids choose the nearest endpoint pair; explicit `w1.start`, `w1.end` or
    /// `w1:2` select a particular fit point. Tangency requires actual curve ends.
    pub fn wire_operand(&self, name: &str) -> Option<Vec<WirePoint>> {
        let at = |wire: &Wire, index: usize| (index < wire.fit.len()).then(|| WirePoint { wire: wire.id.clone(), index });
        if let Some(wire) = self.wire_index(name).and_then(|i| self.wires.get(i)) {
            let first = at(wire, 0)?;
            let last = at(wire, wire.fit.len().checked_sub(1)?)?;
            return Some(if first == last { vec![first] } else { vec![first, last] });
        }
        let (id, end) = name.rsplit_once('.').or_else(|| name.rsplit_once(':'))?;
        let wire = self.wire_index(id).and_then(|i| self.wires.get(i))?;
        let index = match end {
            "start" => 0,
            "end" => wire.fit.len().checked_sub(1)?,
            _ => end.parse().ok()?,
        };
        Some(vec![at(wire, index)?])
    }
    pub fn add_wire_constraint(&mut self, kind: Relation, a: &str, b: &str) -> Result<String> {
        if !matches!(kind, Relation::Coincident | Relation::Tangent) {
            return Err(invalid("use add_wire_dimension for a dimensional relation"));
        }
        let aa = self.wire_operand(a).ok_or_else(|| invalid("select an editable 3D curve endpoint for the first operand"))?;
        let bb = self.wire_operand(b).ok_or_else(|| invalid("3D constraints need two editable spatial curves or points in the same sketch"))?;
        let pair = aa
            .into_iter()
            .flat_map(|a| bb.iter().cloned().map(move |b| (a.clone(), b)))
            .filter(|(a, b)| a.wire != b.wire)
            .filter_map(|(a, b)| Some((point(&self.wires, &a)?.dist(point(&self.wires, &b)?), a, b)))
            .min_by(|a, b| a.0.total_cmp(&b.0));
        let Some((_, a, b)) = pair else {
            return Err(invalid("pick endpoints on two different 3D curves"));
        };
        validate_pair(&self.wires, kind, &a, &b)?;
        if let Some(existing) = self.wire_constraints.iter().find(|c| c.kind == kind && ((c.a == a && c.b == b) || (c.a == b && c.b == a))) {
            return Ok(existing.id.clone());
        }
        if self.wire_constraints.len() >= LIMIT {
            return Err(invalid("a sketch supports at most 256 spatial constraints"));
        }
        let same_direction = kind == Relation::Tangent && tangent(&self.wires, &a).zip(tangent(&self.wires, &b)).is_some_and(|(a, b)| a.dot(b) > 0.);
        let mut trial = self.clone();
        let id = trial.fresh("k3")?;
        trial.wire_constraints.push(WireConstraint { id: id.clone(), kind, a: a.clone(), b, same_direction, value: None, target: None });
        // The first selected curve is the reference while adding the relation.
        let wi = trial.wire_index(&a.wire).ok_or_else(|| invalid("missing 3D reference curve"))?;
        let pins: Vec<_> = trial.wires[wi].fit.iter().enumerate().map(|(p, _)| (wi, p)).collect();
        solve(&mut trial.wires, &trial.wire_constraints, &pins)?;
        *self = trial;
        Ok(id)
    }
    /// Add a driving spatial dimension. A bare line measures its two ends; otherwise
    /// distance/axis dimensions require explicit endpoints. Angles are radians here.
    pub fn add_wire_dimension(&mut self, kind: Relation, a: &str, b: Option<&str>, value: f64) -> Result<String> {
        if matches!(kind, Relation::Coincident | Relation::Tangent | Relation::Fix) {
            return Err(invalid("use the relation or fix command for this constraint"));
        }
        let aa = self.wire_operand(a).ok_or_else(|| invalid("select an editable spatial curve or fit point"))?;
        let first = aa.first().cloned().ok_or_else(|| invalid("missing first spatial point"))?;
        let second = if kind == Relation::Radius {
            first.clone()
        } else if let Some(b) = b {
            self.wire_operand(b).and_then(|v| v.first().cloned()).ok_or_else(|| invalid("select a second editable spatial curve or fit point"))?
        } else if matches!(kind, Relation::Distance | Relation::DeltaX | Relation::DeltaY | Relation::DeltaZ) {
            let w = wire(&self.wires, &first).ok_or_else(|| invalid("missing spatial curve"))?;
            if aa.len() != 2 || w.arc.is_some() || w.fit.len() != 2 {
                return Err(invalid("select one straight 3D line, or two explicit spatial fit points"));
            }
            aa[1].clone()
        } else {
            return Err(invalid("angle needs two straight 3D lines"));
        };
        if self.wire_constraints.len() >= LIMIT {
            return Err(invalid("a sketch supports at most 256 spatial constraints"));
        }
        let mut trial = self.clone();
        let id = trial.fresh("k3")?;
        let c = WireConstraint { id: id.clone(), kind, a: first, b: second, same_direction: false, value: Some(value), target: None };
        validate_constraint(&trial.wires, &c)?;
        seed_parallel_angle(&mut trial.wires, &trial.wire_constraints, &c)?;
        trial.wire_constraints.push(c);
        solve(&mut trial.wires, &trial.wire_constraints, &[])?;
        *self = trial;
        Ok(id)
    }
    pub fn set_wire_dimension(&mut self, id: &str, value: f64) -> Result<()> {
        let mut trial = self.clone();
        let c = trial.wire_constraints.iter_mut().find(|c| c.id == id).ok_or_else(|| invalid("unknown 3D dimension"))?;
        if matches!(c.kind, Relation::Coincident | Relation::Tangent | Relation::Fix) {
            return Err(invalid("this spatial relation has no editable dimension value"));
        }
        c.value = Some(value);
        trial.validate_wire_constraints()?;
        solve(&mut trial.wires, &trial.wire_constraints, &[])?;
        *self = trial;
        Ok(())
    }
    /// A whole curve fixes every editable defining point. A suffixed point fixes only that point.
    pub fn fixed_wire_points(&self, name: &str) -> Result<Vec<WirePoint>> {
        let points = if let Some(w) = self.wire_index(name).and_then(|i| self.wires.get(i)) {
            (0..w.fit.len()).map(|index| WirePoint { wire: w.id.clone(), index }).collect()
        } else {
            self.wire_operand(name).ok_or_else(|| invalid("unknown editable spatial curve or fit point"))?
        };
        if points.is_empty() {
            return Err(invalid("the curve has no editable spatial fit points"));
        }
        for p in &points {
            validate_pair(&self.wires, Relation::Fix, p, p)?;
        }
        Ok(points)
    }
    pub fn fix_wire_geometry(&mut self, name: &str) -> Result<Vec<String>> {
        let points = self.fixed_wire_points(name)?;
        let mut trial = self.clone();
        let mut ids = Vec::new();
        for a in points {
            if let Some(c) = trial.wire_constraints.iter().find(|c| c.kind == Relation::Fix && c.a == a) {
                ids.push(c.id.clone());
                continue;
            }
            if trial.wire_constraints.len() >= LIMIT {
                return Err(invalid("a sketch supports at most 256 spatial constraints"));
            }
            let target = point(&trial.wires, &a).ok_or_else(|| invalid("missing spatial fit point"))?;
            let id = trial.fresh("k3")?;
            trial.wire_constraints.push(WireConstraint {
                id: id.clone(),
                kind: Relation::Fix,
                a: a.clone(),
                b: a,
                same_direction: false,
                value: None,
                target: Some(target),
            });
            ids.push(id);
        }
        trial.validate_wire_constraints()?;
        *self = trial;
        Ok(ids)
    }
    /// Inference at an existing spatial endpoint is a saved relation, not only equal
    /// coordinates. Future point drags and tangent edits therefore keep the junction joined.
    pub(crate) fn connect_wire_ends(&mut self, id: &str) -> Result<()> {
        let Some(wi) = self.wire_index(id) else {
            return Ok(());
        };
        let w = &self.wires[wi];
        let Some(last) = w.fit.len().checked_sub(1) else {
            return Ok(());
        };
        let ends: Vec<_> = if last == 0 { vec![0] } else { vec![0, last] };
        let mut joins = Vec::new();
        for &end in &ends {
            for other in &self.wires {
                if other.id == id || other.link.is_some() || other.space != w.space || other.fit.is_empty() {
                    continue;
                }
                let last = other.fit.len() - 1;
                for target in if last == 0 { vec![0] } else { vec![0, last] } {
                    if w.fit[end].dist(other.fit[target]) < 1e-9 {
                        joins.push((WirePoint { wire: other.id.clone(), index: target }, WirePoint { wire: id.into(), index: end }));
                    }
                }
            }
        }
        if self.wire_constraints.len() + joins.len() > LIMIT {
            return Err(invalid("too many spatial constraints to join this new curve"));
        }
        let previous = self.wire_constraints.len();
        for (a, b) in joins {
            let id = match self.fresh("k3") {
                Ok(id) => id,
                Err(error) => {
                    self.wire_constraints.truncate(previous);
                    return Err(error);
                }
            };
            self.wire_constraints.push(WireConstraint { id, kind: Relation::Coincident, a, b, same_direction: false, value: None, target: None });
        }
        Ok(())
    }
    pub fn solve_wire_constraints(&mut self, hold: Option<(&str, usize)>) -> Result<()> {
        if self.wire_constraints.is_empty() {
            return Ok(());
        }
        self.validate_wire_constraints()?;
        let pins: Vec<_> = hold.and_then(|(id, p)| self.wire_index(id).map(|w| (w, p))).into_iter().collect();
        let mut wires = self.wires.clone();
        solve(&mut wires, &self.wire_constraints, &pins)?;
        self.wires = wires;
        Ok(())
    }
    pub fn validate_wire_constraints(&self) -> Result<()> {
        if self.wire_constraints.len() > LIMIT {
            return Err(invalid("too many spatial constraints"));
        }
        let mut ids = BTreeSet::new();
        for c in &self.wire_constraints {
            if c.id.is_empty() || c.id.len() > 512 || !ids.insert(&c.id) {
                return Err(invalid("invalid spatial constraint id"));
            }
            validate_constraint(&self.wires, c)?;
        }
        Ok(())
    }
}
fn wire<'a>(wires: &'a [Wire], p: &WirePoint) -> Option<&'a Wire> {
    wires.iter().find(|w| w.id == p.wire)
}
fn point(wires: &[Wire], p: &WirePoint) -> Option<Vec3> {
    wire(wires, p)?.fit.get(p.index).copied()
}
fn tangent(wires: &[Wire], p: &WirePoint) -> Option<Vec3> {
    let w = wire(wires, p)?;
    let end = w.fit.len().checked_sub(1)?;
    if p.index != 0 && p.index != end {
        return None;
    }
    let at = *w.fit.get(p.index)?;
    if w.arc.is_some() {
        let [a, m, b] = w.fit.as_slice() else {
            return None;
        };
        let arc = crate::arc_from_3_points(*a, *m, *b)?;
        let t = arc.normal.cross(at - arc.center).normalized()?;
        return Some(if p.index == 0 { t } else { -t });
    }
    if w.fit.len() < 2 || solidcraft_geom::catmull_rom::is_closed(&w.fit) {
        return None;
    }
    let near = if p.index == 0 { *w.fit.get(1)? } else { *w.fit.get(end - 1)? };
    (near - at).normalized()
}
fn validate_pair(wires: &[Wire], kind: Relation, a: &WirePoint, b: &WirePoint) -> Result<()> {
    let (Some(wa), Some(wb)) = (wire(wires, a), wire(wires, b)) else {
        return Err(invalid("a spatial constraint refers to a missing curve"));
    };
    if wa.link.is_some()
        || wb.link.is_some()
        || wa.space != wb.space
        || (a.wire == b.wire && matches!(kind, Relation::Coincident | Relation::Tangent | Relation::Angle))
        || point(wires, a).is_none()
        || point(wires, b).is_none()
    {
        return Err(invalid("spatial constraints require two editable curves in the same coordinate frame"));
    }
    if kind == Relation::Tangent && (tangent(wires, a).is_none() || tangent(wires, b).is_none()) {
        return Err(invalid(
            "3D tangent supports endpoints of lines, circular arcs and open fit splines; pick a curve end, not an interior fit point or closed curve",
        ));
    }
    Ok(())
}
pub(super) fn residual(wires: &[Wire], relations: &[&WireConstraint]) -> Option<Vec<f64>> {
    let mut out = Vec::with_capacity(relations.len() * 6);
    for c in relations {
        let a = point(wires, &c.a)?;
        let b = point(wires, &c.b)?;
        let d = b - a;
        match c.kind {
            Relation::Coincident | Relation::Tangent => {
                out.extend([d.x, d.y, d.z]);
                if c.kind == Relation::Tangent {
                    let d = tangent(wires, &c.a)? - tangent(wires, &c.b)? * if c.same_direction { 1. } else { -1. };
                    out.extend([d.x, d.y, d.z]);
                }
            }
            Relation::Fix => {
                let d = a - c.target?;
                out.extend([d.x, d.y, d.z]);
            }
            _ => out.push(measure(wires, c.kind, &c.a, &c.b)? - c.value?),
        }
    }
    out.iter().all(|x| x.is_finite()).then_some(out)
}
/// Measurement uses the stored wire coordinate frame, never the viewing camera.
pub(super) fn measure(wires: &[Wire], kind: Relation, a: &WirePoint, b: &WirePoint) -> Option<f64> {
    let d = point(wires, b)? - point(wires, a)?;
    match kind {
        Relation::Distance => Some(d.len()),
        Relation::DeltaX => Some(d.x),
        Relation::DeltaY => Some(d.y),
        Relation::DeltaZ => Some(d.z),
        Relation::Radius => {
            let [a, m, b] = wire(wires, a)?.fit.as_slice() else {
                return None;
            };
            Some(crate::arc_from_3_points(*a, *m, *b)?.radius)
        }
        Relation::Angle => {
            let direction = |p: &WirePoint| {
                let w = wire(wires, p)?;
                if w.arc.is_some() || w.fit.len() != 2 {
                    return None;
                }
                (w.fit[1] - w.fit[0]).normalized()
            };
            let (a, b) = (direction(a)?, direction(b)?);
            Some(a.cross(b).len().atan2(a.dot(b)))
        }
        _ => None,
    }
}
fn validate_constraint(wires: &[Wire], c: &WireConstraint) -> Result<()> {
    validate_pair(wires, c.kind, &c.a, &c.b)?;
    match c.kind {
        Relation::Coincident | Relation::Tangent => {}
        Relation::Fix => {
            let Some(p) = c.target else {
                return Err(invalid("fixed spatial point is missing its target"));
            };
            if ![p.x, p.y, p.z].iter().all(|v| v.is_finite() && v.abs() < crate::MAX_COORD) {
                return Err(invalid("invalid fixed spatial point"));
            }
        }
        kind => {
            let Some(v) = c.value.filter(|v| v.is_finite() && v.abs() < crate::MAX_COORD) else {
                return Err(invalid("a 3D dimension needs a finite value within coordinate limits"));
            };
            if matches!(kind, Relation::Distance | Relation::Radius) && v <= 1e-7 {
                return Err(invalid("3D distance and radius must be positive"));
            }
            if kind == Relation::Angle && !(1e-5..std::f64::consts::PI - 1e-5).contains(&v) {
                return Err(invalid("3D line angle must be strictly between 0 and 180 degrees"));
            }
            if kind == Relation::Radius && !wire(wires, &c.a).is_some_and(|w| w.arc.is_some()) {
                return Err(invalid("3D radius needs an editable circular arc"));
            }
            if matches!(kind, Relation::Distance | Relation::DeltaX | Relation::DeltaY | Relation::DeltaZ) && c.a == c.b {
                return Err(invalid("select two different spatial fit points"));
            }
            if measure(wires, kind, &c.a, &c.b).is_none() {
                return Err(invalid("unsupported or degenerate 3D dimension geometry"));
            }
        }
    }
    Ok(())
}
pub(super) fn affected_points(wires: &[Wire], relations: &[&WireConstraint]) -> Result<BTreeSet<(usize, usize)>> {
    let mut points = BTreeSet::new();
    for c in relations {
        for p in [&c.a, &c.b] {
            let wi = wires.iter().position(|w| w.id == p.wire).ok_or_else(|| invalid("missing spatial curve"))?;
            points.insert((wi, p.index));
            let w = &wires[wi];
            if c.kind == Relation::Radius || (c.kind == Relation::Tangent && w.arc.is_some()) {
                points.extend((0..w.fit.len()).map(|i| (wi, i)));
            } else if c.kind == Relation::Angle {
                points.extend((0..2).map(|i| (wi, i)));
            } else if c.kind == Relation::Tangent {
                points.insert((wi, if p.index == 0 { 1 } else { w.fit.len() - 2 }));
            }
        }
    }
    Ok(points)
}

fn solve(wires: &mut [Wire], relations: &[WireConstraint], pins: &[(usize, usize)]) -> Result<()> {
    let mut pending: BTreeSet<usize> = (0..relations.len()).collect();
    while let Some(&first) = pending.first() {
        let mut names = BTreeSet::from([relations[first].a.wire.clone(), relations[first].b.wire.clone()]);
        let mut group = BTreeSet::new();
        loop {
            let more: Vec<_> =
                pending.iter().copied().filter(|i| names.contains(&relations[*i].a.wire) || names.contains(&relations[*i].b.wire)).collect();
            if more.is_empty() {
                break;
            }
            for i in more {
                pending.remove(&i);
                group.insert(i);
                names.insert(relations[i].a.wire.clone());
                names.insert(relations[i].b.wire.clone());
            }
        }
        let group: Vec<_> = group.into_iter().map(|i| &relations[i]).collect();
        solve_group(wires, &group, pins)?;
    }
    Ok(())
}
fn solve_group(wires: &mut [Wire], relations: &[&WireConstraint], pins: &[(usize, usize)]) -> Result<()> {
    let fail = || invalid("3D constraints conflict with this edit or could not converge; no geometry was changed");
    let mut r = residual(wires, relations).ok_or_else(fail)?;
    if r.iter().all(|v| v.abs() < TOL) {
        return Ok(());
    }
    let mut points = affected_points(wires, relations)?;
    points.retain(|p| !pins.contains(p));
    if points.len() > 96 {
        return Err(invalid("this connected 3D constraint set exceeds 96 editable fit points; split it into smaller sketches"));
    }
    let vars: Vec<_> = points.iter().flat_map(|&(w, p)| (0..3).map(move |axis| (w, p, axis))).collect();
    if vars.is_empty() {
        return Err(fail());
    }
    let get = |wires: &[Wire], (w, p, a): (usize, usize, usize)| {
        let v = wires[w].fit[p];
        [v.x, v.y, v.z][a]
    };
    let set = |wires: &mut [Wire], (w, p, a): (usize, usize, usize), value: f64| {
        let v = &mut wires[w].fit[p];
        match a {
            0 => v.x = value,
            1 => v.y = value,
            _ => v.z = value,
        }
    };
    let mut lambda = 1e-5;
    let mut solved = false;
    for _ in 0..80 {
        let n = vars.len();
        let m = r.len();
        let mut j = vec![0.; n * m];
        for (col, &v) in vars.iter().enumerate() {
            let x = get(wires, v);
            let h = 1e-5_f64.max(x.abs() * 1e-8);
            set(wires, v, x + h);
            let perturbed = residual(wires, relations);
            set(wires, v, x);
            let Some(perturbed) = perturbed else {
                return Err(fail());
            };
            for row in 0..m {
                j[row * n + col] = (perturbed[row] - r[row]) / h;
            }
        }
        let mut a = crate::linalg::Mat::zeros(n, n);
        let mut b = vec![0.; n];
        for col in 0..n {
            for row in 0..m {
                b[col] -= j[row * n + col] * r[row];
            }
            for k in 0..=col {
                let value = (0..m).map(|row| j[row * n + col] * j[row * n + k]).sum::<f64>();
                a.set(col, k, value);
                a.set(k, col, value);
            }
            a.add(col, col, lambda);
        }
        let Some(step) = crate::linalg::solve(a, b) else {
            return Err(fail());
        };
        let saved: Vec<_> = vars.iter().map(|&v| get(wires, v)).collect();
        for ((&v, &x), &d) in vars.iter().zip(&saved).zip(&step) {
            set(wires, v, x + d);
        }
        let next = residual(wires, relations).filter(|_| {
            vars.iter().all(|&v| {
                let x = get(wires, v);
                x.is_finite() && x.abs() < crate::MAX_COORD
            })
        });
        if let Some(next) = next.filter(|next| next.iter().map(|v| v * v).sum::<f64>() < r.iter().map(|v| v * v).sum::<f64>()) {
            r = next;
            lambda = (lambda * 0.25).max(1e-12);
            if r.iter().all(|v| v.abs() < TOL) {
                solved = true;
                break;
            }
        } else {
            for (&v, &x) in vars.iter().zip(&saved) {
                set(wires, v, x);
            }
            lambda *= 10.;
            if lambda > 1e10 {
                break;
            }
        }
    }
    if !solved {
        return Err(fail());
    }
    for wi in points.iter().map(|(w, _)| *w).collect::<BTreeSet<_>>() {
        let w = &mut wires[wi];
        if w.arc.is_some() {
            let [a, m, b] = w.fit.as_slice() else {
                return Err(fail());
            };
            let arc = crate::arc_from_3_points(*a, *m, *b).ok_or_else(fail)?;
            w.pts = crate::arc_polyline(&arc, 1e-3);
            w.arc = Some(arc);
        } else {
            if w.fit.windows(2).any(|p| p[0].dist(p[1]) < 1e-9) {
                return Err(fail());
            }
            w.pts = crate::fit_polyline(&w.fit);
        }
    }
    Ok(())
}

/// The unsigned line angle has a cusp at parallel/antiparallel directions. A
/// deterministic geometric seed avoids a derivative step that only translates
/// both endpoints. This is a starting guess, never a replacement for solving
/// and verifying every persistent dimension and fixed-point target.
fn seed_parallel_angle(wires: &mut [Wire], relations: &[WireConstraint], dimension: &WireConstraint) -> Result<()> {
    if dimension.kind != Relation::Angle {
        return Ok(());
    }
    let (Some(ai), Some(bi)) = (wires.iter().position(|w| w.id == dimension.a.wire), wires.iter().position(|w| w.id == dimension.b.wire)) else {
        return Err(invalid("missing angular dimension curve"));
    };
    let direction = |w: &Wire| {
        let [a, b] = w.fit.as_slice() else {
            return None;
        };
        (*b - *a).normalized()
    };
    let (Some(a), Some(b)) = (direction(&wires[ai]), direction(&wires[bi])) else {
        return Err(invalid("angle needs two nondegenerate straight lines"));
    };
    if a.cross(b).len() > 1e-6 {
        return Ok(());
    }
    let fixed = |wi: usize, index: usize| relations.iter().any(|c| c.kind == Relation::Fix && c.a.wire == wires[wi].id && c.a.index == index);
    let b_fixed = fixed(bi, 0) && fixed(bi, 1);
    let a_fixed = fixed(ai, 0) && fixed(ai, 1);
    let (reference, moving) = if b_fixed && !a_fixed { (bi, ai) } else { (ai, bi) };
    let first_fixed = fixed(moving, 0);
    let last_fixed = fixed(moving, 1);
    let Some(u) = direction(&wires[reference]) else {
        return Err(invalid("invalid angular reference"));
    };
    let Some(v) = direction(&wires[moving]) else {
        return Err(invalid("invalid angular curve"));
    };
    let axis = if u.x.abs() < 0.8 { Vec3::X } else { Vec3::Y };
    let separation = wires[moving].fit[0] - wires[reference].fit[0];
    let perpendicular = (v - u * u.dot(v))
        .normalized()
        .or_else(|| (separation - u * u.dot(separation)).normalized())
        .or_else(|| (axis - u * u.dot(axis)).normalized())
        .ok_or_else(|| invalid("could not establish an angular seed plane"))?;
    let angle = dimension.value.ok_or_else(|| invalid("missing angle"))?;
    let direction = u * angle.cos() + perpendicular * angle.sin();
    let [a, b] = wires[moving].fit.as_slice() else {
        return Err(invalid("angle needs straight lines"));
    };
    let (a, b) = (*a, *b);
    let length = a.dist(b);
    let (start, end) = if first_fixed {
        (a, a + direction * length)
    } else if last_fixed {
        (b - direction * length, b)
    } else {
        let center = (a + b) * 0.5;
        (center - direction * (length * 0.5), center + direction * (length * 0.5))
    };
    if !crate::model::in_range3(start) || !crate::model::in_range3(end) {
        return Err(invalid("angular seed would exceed spatial coordinate limits"));
    }
    wires[moving].fit = vec![start, end];
    wires[moving].pts = crate::fit_polyline(&wires[moving].fit);
    Ok(())
}
