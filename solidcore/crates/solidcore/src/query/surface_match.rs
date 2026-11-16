//! Exact matching of polynomial fitted patches along straight UV trim segments.
//!
//! Original construction from Bernstein derivative identities (Farin/Hansford,
//! Essentials of CAGD, chapter6). An affine edge frame raises bicubic tensor degree
//! to at most six. Boundary jets are imposed as polynomial identities, including
//! the affine reparameterization; sampled normal/curvature checks are diagnostics,
//! not the proof of continuity. A positive Bernstein bound on the projected
//! Jacobian proves regularity on the complete resulting parameter rectangle.
//! Bounded to4096 trim edges, 7x7 surface coefficients and degree12 edge curves.
use super::scan_surface_fit::{ordered_uv_loops, trimmed_sheet};
use crate::curve::curve::Curve;
use crate::nurbs::{
    b_spline_curve::BSplineCurve, b_spline_surface::BSplineSurface, crv_basic::Curve as NurbCurve,
    math::CPoint, srf_basic::Surface as NurbSurface,
};
use crate::surface::surface::Surface;
use crate::{Brep, Kernel, Result, SolidError, Vec3};
use serde::Serialize;
const N: usize = 6;
fn invalid(reason: &'static str) -> SolidError {
    SolidError::invalid_input("query::surface_match", reason)
}
/// Requested geometric continuity, supplied by exact affine-parametric jet matching.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchOrder {
    /// Position continuity.
    G0,
    /// Position and tangent-plane continuity.
    G1,
    /// Position, tangent-plane and curvature continuity.
    G2,
}
/// Controls for the moving patch. The neighbour is immutable.
#[derive(Clone, Copy, Debug)]
pub struct MatchOptions {
    /// Requested jet order.
    pub order: MatchOrder,
    /// Positive scale of the affine cross-boundary parameter map (at most100).
    pub cross_scale: f64,
    /// Reverse neighbour edge direction to pair the boundary endpoints.
    pub reverse_target: bool,
}
impl Default for MatchOptions {
    fn default() -> Self {
        Self {
            order: MatchOrder::G2,
            cross_scale: 1.,
            reverse_target: true,
        }
    }
}
/// One straight UV trim segment, retaining its source record edge identity.
#[derive(Clone, Debug)]
pub struct TrimSegment {
    /// Record edge index used for explicit selection.
    pub edge_index: usize,
    /// Polynomial pcurve control points; their complete polygon must be collinear and monotone.
    pub points: Vec<[f64; 2]>,
    /// Clamped polynomial pcurve knot vector.
    pub knots: Vec<f64>,
    /// Active edge interval in the pcurve parameter.
    pub interval: [f64; 2],
    /// Whether the boundary follows the increasing curve parameter.
    pub forward: bool,
}
/// Bicubic source data. Only nonrational, clamped, unit-domain fitted patches are accepted.
#[derive(Clone, Debug)]
pub struct PatchInput {
    /// Nonrational bicubic control net, U rows then V columns.
    pub points: Vec<Vec<[f64; 3]>>,
    /// Clamped cubic U knots on [0,1].
    pub u_knots: Vec<f64>,
    /// Clamped cubic V knots on [0,1].
    pub v_knots: Vec<f64>,
    /// Canonical outer CCW loop followed by CW holes.
    pub loops: Vec<Vec<TrimSegment>>,
}
/// Algebraic bounds and independent sampled seam diagnostics.
#[derive(Clone, Debug, Serialize)]
pub struct MatchReport {
    /// Requested geometric continuity name.
    pub requested_order: String,
    /// Explanation of the algebraic guarantee and diagnostics.
    pub construction: String,
    /// Convex-hull bound on the boundary position residual.
    pub seam_gap_bound_mm: f64,
    /// Bound on the first cross-parameter derivative residual (mm per edge-frame unit).
    pub first_jet_bound: f64,
    /// Bound on the second cross-parameter derivative residual.
    pub second_jet_bound: f64,
    /// Largest measured seam gap at the stated samples.
    pub max_gap_mm: f64,
    /// Largest measured unsigned tangent-plane angle.
    pub max_normal_angle_deg: f64,
    /// Largest sampled Frobenius norm of the shape-operator difference in a shared orthonormal frame.
    pub max_curvature_jump_per_mm: f64,
    /// Number of independent seam diagnostic stations.
    pub samples: usize,
    /// Resulting U control count.
    pub control_points_u: usize,
    /// Resulting V control count.
    pub control_points_v: usize,
    /// Retained source outer and hole loops.
    pub boundary_loops: usize,
    /// Actual native output trim edge count.
    pub boundary_edges: usize,
    /// Control-hull displacement bound over the source edge-frame rectangle.
    pub source_displacement_bound_mm: f64,
    /// True when a positive Bernstein Jacobian bound verifies regularity over the parameter rectangle.
    pub regularity_verified: bool,
}
#[derive(Clone)]
struct Poly {
    c: Vec<Vec3>,
}
impl Poly {
    fn zero() -> Self {
        Self {
            c: vec![Vec3::ZERO; 49],
        }
    }
    fn get(&self, i: usize, j: usize) -> Result<Vec3> {
        self.c
            .get(i * 7 + j)
            .copied()
            .ok_or_else(|| invalid("polynomial index exceeds its degree"))
    }
    fn add(&mut self, i: usize, j: usize, p: Vec3) -> Result<()> {
        let v = self
            .c
            .get_mut(i * 7 + j)
            .ok_or_else(|| invalid("polynomial index exceeds its degree"))?;
        *v += p;
        Ok(())
    }
    fn eval(&self, s: f64, r: f64, ds: usize, dr: usize) -> Result<Vec3> {
        let mut out = Vec3::ZERO;
        for i in ds..=N {
            for j in dr..=N {
                let a =
                    fall(i, ds) * fall(j, dr) * s.powi((i - ds) as i32) * r.powi((j - dr) as i32);
                out += self.get(i, j)? * a;
            }
        }
        Ok(out)
    }
}
fn fall(n: usize, k: usize) -> f64 {
    (0..k).map(|i| (n - i) as f64).product()
}
fn choose(n: usize, k: usize) -> f64 {
    if k > n {
        return 0.;
    }
    let mut v = 1.;
    for i in 0..k {
        v *= ((n - i) as f64) / ((i + 1) as f64);
    }
    v
}
fn scalar_power(a: [f64; 3], n: usize) -> Vec<(usize, usize, f64)> {
    let mut out = Vec::new();
    for i in 0..=n {
        for j in 0..=n - i {
            out.push((
                i,
                j,
                choose(n, i)
                    * choose(n - i, j)
                    * a[0].powi((n - i - j) as i32)
                    * a[1].powi(i as i32)
                    * a[2].powi(j as i32),
            ));
        }
    }
    out
}
fn composed(input: &PatchInput, a: [f64; 2], d: [f64; 2], n: [f64; 2]) -> Result<Poly> {
    if input.points.len() != 4 || input.points.iter().any(|r| r.len() != 4) {
        return Err(invalid("match requires bicubic fitted patches; edit the original Match feature to adjust an existing join; multi-edge constraint networks are not supported"));
    }
    let expected = [0., 0., 0., 0., 1., 1., 1., 1.];
    if input.u_knots != expected || input.v_knots != expected {
        return Err(invalid(
            "match requires a clamped bicubic fitted patch on its unit parameter domain",
        ));
    }
    let mut uv = Poly::zero();
    for (i, row) in input.points.iter().enumerate() {
        for (j, p) in row.iter().enumerate() {
            if !p.iter().all(|x| x.is_finite() && x.abs() < 1e9) {
                return Err(invalid("nonfinite or excessive patch coordinates"));
            }
            for u in i..4 {
                for v in j..4 {
                    let sign = if (u - i + v - j) % 2 == 0 { 1. } else { -1. };
                    uv.add(
                        u,
                        v,
                        Vec3::from_array(*p)
                            * (choose(3, i)
                                * choose(3 - i, u - i)
                                * choose(3, j)
                                * choose(3 - j, v - j)
                                * sign),
                    )?;
                }
            }
        }
    }
    let mut out = Poly::zero();
    for u in 0..4 {
        for v in 0..4 {
            let p = uv.get(u, v)?;
            for (i, j, x) in scalar_power([a[0], d[0], n[0]], u) {
                for (k, l, y) in scalar_power([a[1], d[1], n[1]], v) {
                    out.add(i + k, j + l, p * (x * y))?;
                }
            }
        }
    }
    Ok(out)
}
#[derive(Clone, Copy)]
struct Frame {
    a: [f64; 2],
    d: [f64; 2],
    n: [f64; 2],
    length2: f64,
}
impl Frame {
    fn of(a: [f64; 2], b: [f64; 2]) -> Result<Self> {
        let d = [b[0] - a[0], b[1] - a[1]];
        let length2 = d[0] * d[0] + d[1] * d[1];
        if !length2.is_finite() || length2 < 1e-12 {
            return Err(invalid(
                "selected trim edge is too short for a stable edge frame",
            ));
        }
        Ok(Self {
            a,
            d,
            n: [-d[1], d[0]],
            length2,
        })
    }
    fn project(self, p: [f64; 2]) -> [f64; 2] {
        let x = p[0] - self.a[0];
        let y = p[1] - self.a[1];
        [
            (x * self.d[0] + y * self.d[1]) / self.length2,
            (x * self.n[0] + y * self.n[1]) / self.length2,
        ]
    }
}
fn boundary(
    _k: &Kernel,
    input: &PatchInput,
    selected: usize,
) -> Result<(Vec<Vec<[f64; 2]>>, Frame)> {
    if input.loops.is_empty() || input.loops.iter().map(Vec::len).sum::<usize>() > 4096 {
        return Err(invalid(
            "match supports at most4096 straight trim edges on one face",
        ));
    }
    let mut loops = Vec::new();
    let mut picked = None;
    for ring in &input.loops {
        let mut points = Vec::new();
        let mut ends = Vec::new();
        for edge in ring {
            let knots = &edge.knots;
            let t = edge.interval;
            if edge.points.len() < 2
                || edge.points.len() > 128
                || knots.len() > 141
                || !knots
                    .iter()
                    .chain(t.iter())
                    .chain(edge.points.iter().flatten())
                    .all(|x| x.is_finite())
            {
                return Err(invalid(
                    "trim pcurve exceeds the bounded polynomial budget or has nonfinite values",
                ));
            }
            let degree = knots
                .len()
                .checked_sub(edge.points.len() + 1)
                .filter(|n| (1..=12).contains(n))
                .ok_or_else(|| invalid("invalid polynomial trim degree"))?;
            let low = *knots
                .get(degree)
                .ok_or_else(|| invalid("missing trim domain"))?;
            let high = *knots
                .get(edge.points.len())
                .ok_or_else(|| invalid("missing trim domain"))?;
            if t[0] < low || t[1] > high || t[1] <= t[0] {
                return Err(invalid("invalid trim interval"));
            }
            let first = *edge
                .points
                .first()
                .ok_or_else(|| invalid("empty trim control polygon"))?;
            let last = *edge
                .points
                .last()
                .ok_or_else(|| invalid("empty trim control polygon"))?;
            let d = [last[0] - first[0], last[1] - first[1]];
            let length = d[0].hypot(d[1]);
            if length < 1e-12 {
                return Err(invalid("collapsed trim control polygon"));
            }
            let scale = edge
                .points
                .iter()
                .flatten()
                .map(|x| x.abs())
                .fold(1., f64::max);
            let roundoff = 2048. * f64::EPSILON * scale * length;
            let mut previous = f64::NEG_INFINITY;
            for p in &edge.points {
                let x = p[0] - first[0];
                let y = p[1] - first[1];
                let along = x * d[0] + y * d[1];
                if (x * d[1] - y * d[0]).abs() > roundoff || along < previous - roundoff {
                    return Err(invalid("matching supports straight monotone UV trim segments; this pcurve is curved or retraces"));
                }
                previous = along;
            }
            let curve_points = edge
                .points
                .iter()
                .map(|p| CPoint::from_point(Vec3::new(p[0], p[1], 0.)))
                .collect();
            let curve = BSplineCurve::from_gw_nurb(
                2,
                &NurbCurve::new(curve_points, degree, knots.clone())?,
            );
            let at = |t: f64| -> Result<[f64; 2]> {
                let value = curve.evaluate(t, 0, true, false)?;
                let p = value
                    .first()
                    .ok_or_else(|| invalid("missing evaluated trim endpoint"))?;
                Ok([p.x, p.y])
            };
            let (mut a, mut b) = (at(t[0])?, at(t[1])?);
            if !edge.forward {
                std::mem::swap(&mut a, &mut b);
            }
            if edge.edge_index == selected {
                if picked.is_some() {
                    return Err(invalid(
                        "select one unambiguous trim edge, not a periodic seam",
                    ));
                }
                picked = Some(Frame::of(a, b)?);
            }
            points.push(a);
            ends.push(b);
        }
        for (a, b) in ends
            .iter()
            .zip(points.iter().cycle().skip(1))
            .take(points.len())
        {
            if (a[0] - b[0]).abs().max((a[1] - b[1]).abs()) > 1e-10 {
                return Err(invalid("trim loop has an open parameter-space gap"));
            }
        }
        loops.push(points);
    }
    let frame =
        picked.ok_or_else(|| invalid("selected edge does not belong to the fitted face"))?;
    // Canonical record loops already run with the material to the left. Do not silently
    // reverse the selected frame when an invalid imported record has another convention.
    for (i, ring) in loops.iter().enumerate() {
        let area = ring
            .iter()
            .zip(ring.iter().cycle().skip(1))
            .take(ring.len())
            .map(|(a, b)| a[0] * b[1] - a[1] * b[0])
            .sum::<f64>();
        if (i == 0 && area <= 0.) || (i > 0 && area >= 0.) {
            return Err(invalid(
                "trim loop orientation must be outer counterclockwise and holes clockwise",
            ));
        }
    }
    Ok((ordered_uv_loops(loops)?, frame))
}
fn normalize(p: &Poly, min: [f64; 2], width: [f64; 2]) -> Result<Poly> {
    let mut out = Poly::zero();
    for i in 0..=N {
        for j in 0..=N {
            let c = p.get(i, j)?;
            for u in 0..=i {
                for v in 0..=j {
                    out.add(
                        u,
                        v,
                        c * (choose(i, u)
                            * min[0].powi((i - u) as i32)
                            * width[0].powi(u as i32)
                            * choose(j, v)
                            * min[1].powi((j - v) as i32)
                            * width[1].powi(v as i32)),
                    )?;
                }
            }
        }
    }
    Ok(out)
}
fn net(p: &Poly) -> Result<Vec<Vec<Vec3>>> {
    let mut out = Vec::new();
    for i in 0..=N {
        let mut row = Vec::new();
        for j in 0..=N {
            let mut x = Vec3::ZERO;
            for u in 0..=i {
                for v in 0..=j {
                    x += p.get(u, v)? * (choose(i, u) / choose(N, u) * choose(j, v) / choose(N, v));
                }
            }
            row.push(x);
        }
        out.push(row);
    }
    Ok(out)
}
fn power_from_net(control: &[Vec<Vec3>]) -> Result<Poly> {
    let mut p = Poly::zero();
    for (i, row) in control.iter().enumerate() {
        for (j, point) in row.iter().enumerate() {
            for u in i..=N {
                for v in j..=N {
                    let sign = if (u - i + v - j) % 2 == 0 { 1. } else { -1. };
                    p.add(
                        u,
                        v,
                        *point
                            * (choose(N, i)
                                * choose(N - i, u - i)
                                * choose(N, j)
                                * choose(N - j, v - j)
                                * sign),
                    )?;
                }
            }
        }
    }
    Ok(p)
}
fn edge_curve(p: &Poly, a: [f64; 2], b: [f64; 2]) -> Result<Curve> {
    let mut power = vec![Vec3::ZERO; 13];
    for i in 0..=N {
        for j in 0..=N {
            for u in 0..=i {
                for v in 0..=j {
                    let coefficient = choose(i, u)
                        * a[0].powi((i - u) as i32)
                        * (b[0] - a[0]).powi(u as i32)
                        * choose(j, v)
                        * a[1].powi((j - v) as i32)
                        * (b[1] - a[1]).powi(v as i32);
                    *power
                        .get_mut(u + v)
                        .ok_or_else(|| invalid("edge degree exceeded"))? +=
                        p.get(i, j)? * coefficient;
                }
            }
        }
    }
    let mut points = Vec::new();
    for i in 0..=12 {
        let mut v = Vec3::ZERO;
        for (j, c) in power.iter().enumerate().take(i + 1) {
            v += *c * (choose(i, j) / choose(12, j));
        }
        points.push(CPoint::from_point(v));
    }
    let mut knots = vec![0.; 13];
    knots.extend([1.; 13]);
    Ok(BSplineCurve::from_gw_nurb(3, &NurbCurve::new(points, 12, knots)?).into())
}
fn jet_bound(a: &Poly, b: &Poly, j: usize) -> Result<f64> {
    let mut largest = 0.0_f64;
    for i in 0..=N {
        let mut c = Vec3::ZERO;
        for k in 0..=i {
            c += (a.get(k, j)? - b.get(k, j)?) * (fall(j, j) * choose(i, k) / choose(N, k));
        }
        largest = largest.max(c.length());
    }
    Ok(largest)
}
fn regular(p: &Poly) -> Result<bool> {
    // Work from the final Bernstein net: differentiation and Bernstein products
    // avoid cancellation from converting a high-degree Jacobian through powers.
    let control = net(p)?;
    let at = |i: usize, j: usize| {
        control
            .get(i)
            .and_then(|r| r.get(j))
            .copied()
            .ok_or_else(|| invalid("missing regularity control point"))
    };
    let mut du = Vec::new();
    let mut dv = Vec::new();
    let mut mid_u = Vec3::ZERO;
    let mut mid_v = Vec3::ZERO;
    for i in 0..6 {
        for j in 0..7 {
            let v = (at(i + 1, j)? - at(i, j)?) * 6.;
            mid_u += v * (choose(5, i) * choose(6, j) / 2048.);
            du.push((i, j, v));
        }
    }
    for i in 0..7 {
        for j in 0..6 {
            let v = (at(i, j + 1)? - at(i, j)?) * 6.;
            mid_v += v * (choose(6, i) * choose(5, j) / 2048.);
            dv.push((i, j, v));
        }
    }
    let direction = mid_u.cross(mid_v).unitize()?;
    let mut coefficients = vec![(0.0_f64, 0.0_f64); 144];
    for &(i, j, u) in &du {
        for &(k, l, v) in &dv {
            let weight =
                choose(5, i) * choose(6, k) / choose(11, i + k) * choose(6, j) * choose(5, l)
                    / choose(11, j + l);
            let term = u.cross(v).dot(direction) * weight;
            let entry = coefficients
                .get_mut((i + k) * 12 + j + l)
                .ok_or_else(|| invalid("Jacobian degree exceeded"))?;
            entry.0 += term;
            // Absolute-product bound includes the determinant's own cancellation.
            entry.1 += weight * (u.length() * v.length());
        }
    }
    let scale = coefficients.iter().map(|v| v.1).fold(1., f64::max);
    Ok(coefficients
        .iter()
        .all(|(value, sum)| *value - 256. * f64::EPSILON * sum > scale * 1e-12))
}
fn curvature(p: &Poly, s: f64, r: f64, e1: Vec3, e2: Vec3, n: Vec3) -> Result<[f64; 3]> {
    let a = p.eval(s, r, 1, 0)?;
    let b = p.eval(s, r, 0, 1)?;
    let aa = a.dot(a);
    let ab = a.dot(b);
    let bb = b.dot(b);
    let det = aa * bb - ab * ab;
    if det <= 1e-20 {
        return Err(invalid("singular seam tangent plane"));
    }
    let coords = |e: Vec3| {
        [
            (e.dot(a) * bb - e.dot(b) * ab) / det,
            (e.dot(b) * aa - e.dot(a) * ab) / det,
        ]
    };
    let x = coords(e1);
    let y = coords(e2);
    let ll = p.eval(s, r, 2, 0)?.dot(n);
    let mm = p.eval(s, r, 1, 1)?.dot(n);
    let nn = p.eval(s, r, 0, 2)?.dot(n);
    let form = |x: [f64; 2], y: [f64; 2]| {
        ll * x[0] * y[0] + mm * (x[0] * y[1] + x[1] * y[0]) + nn * x[1] * y[1]
    };
    Ok([form(x, x), form(x, y), form(y, y)])
}
/// Match one source trim segment to a fixed neighbour segment, retaining every source loop.
pub fn match_trimmed(
    k: &mut Kernel,
    source: &PatchInput,
    source_edge: usize,
    target: &PatchInput,
    target_edge: usize,
    options: MatchOptions,
) -> Result<(Brep, MatchReport)> {
    if !options.cross_scale.is_finite() || options.cross_scale <= 0. || options.cross_scale > 100. {
        return Err(invalid(
            "cross scale must be positive and no greater than100",
        ));
    }
    let (loops, sf) = boundary(k, source, source_edge)?;
    let (_, tf) = boundary(k, target, target_edge)?;
    let original = composed(source, sf.a, sf.d, sf.n)?;
    let (ta, td) = if options.reverse_target {
        ([tf.a[0] + tf.d[0], tf.a[1] + tf.d[1]], [-tf.d[0], -tf.d[1]])
    } else {
        (tf.a, tf.d)
    };
    let neighbour = composed(
        target,
        ta,
        td,
        [
            -tf.n[0] * options.cross_scale,
            -tf.n[1] * options.cross_scale,
        ],
    )?;
    let projected: Vec<Vec<_>> = loops
        .iter()
        .map(|l| l.iter().map(|p| sf.project(*p)).collect())
        .collect();
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for p in projected.iter().flatten() {
        for axis in 0..2 {
            if let (Some(a), Some(b), Some(v)) = (min.get_mut(axis), max.get_mut(axis), p.get(axis))
            {
                *a = a.min(*v);
                *b = b.max(*v);
            }
        }
    }
    let far = max[1];
    let width = [max[0] - min[0], max[1] - min[1]];
    if far <= 1e-6 || width.iter().any(|v| !v.is_finite() || *v <= 1e-8) {
        return Err(invalid("selected edge has no stable inward patch extent"));
    }
    let order = match options.order {
        MatchOrder::G0 => 0,
        MatchOrder::G1 => 1,
        MatchOrder::G2 => 2,
    };
    let mut matched = original.clone();
    for i in 0..=N {
        for j in 0..=order {
            let delta = neighbour.get(i, j)? - original.get(i, j)?;
            matched.add(i, j, delta)?;
            matched.add(i, 3, delta * (-far.powi(j as i32 - 3)))?;
        }
    }
    let normalized = normalize(&matched, min, width)?;
    if !regular(&normalized)? {
        return Err(invalid("matching would create an unverified or folded patch; lower cross scale or select a cleaner neighbouring pair"));
    }
    let control = net(&normalized)?;
    // Verify the native control net after its power-to-Bernstein rounding.
    // Ill-conditioned edge frames must refuse instead of grading an ideal seam.
    let native_power = power_from_net(&control)?;
    let actual = normalize(
        &native_power,
        [-min[0] / width[0], -min[1] / width[1]],
        [1. / width[0], 1. / width[1]],
    )?;
    let mut report=MatchReport{requested_order:format!("{:?}",options.order).to_lowercase(),construction:"Exact polynomial jets under an invertible affine edge reparameterization; Bernstein projected-Jacobian bound proves regularity. Normals and full curvature tensor are sampled diagnostics.".into(),seam_gap_bound_mm:jet_bound(&actual,&neighbour,0)?,first_jet_bound:jet_bound(&actual,&neighbour,1)?,second_jet_bound:jet_bound(&actual,&neighbour,2)?,max_gap_mm:0.,max_normal_angle_deg:0.,max_curvature_jump_per_mm:0.,samples:65,control_points_u:7,control_points_v:7,boundary_loops:loops.len(),boundary_edges:0,source_displacement_bound_mm:0.,regularity_verified:true};
    for i in 0..65 {
        let s = i as f64 / 64.;
        let p = actual.eval(s, 0., 0, 0)?;
        let q = neighbour.eval(s, 0., 0, 0)?;
        let a = actual.eval(s, 0., 1, 0)?;
        let b = actual.eval(s, 0., 0, 1)?;
        let n = a.cross(b).unitize()?;
        let mut m = neighbour
            .eval(s, 0., 1, 0)?
            .cross(neighbour.eval(s, 0., 0, 1)?)
            .unitize()?;
        if n.dot(m) < 0. {
            m = -m;
        }
        report.max_gap_mm = report.max_gap_mm.max((p - q).length());
        report.max_normal_angle_deg = report
            .max_normal_angle_deg
            .max(n.cross(m).length().atan2(n.dot(m)).to_degrees());
        let e1 = a.unitize()?;
        let e2 = n.cross(e1);
        let c = curvature(&actual, s, 0., e1, e2, n)?;
        let d = curvature(&neighbour, s, 0., e1, e2, m)?;
        let jump =
            ((c[0] - d[0]).powi(2) + 2. * (c[1] - d[1]).powi(2) + (c[2] - d[2]).powi(2)).sqrt();
        report.max_curvature_jump_per_mm = report.max_curvature_jump_per_mm.max(jump);
    }
    let scale = source
        .points
        .iter()
        .flatten()
        .flatten()
        .map(|v| v.abs())
        .fold(1., f64::max);
    let gate = (scale * 1e-9).min(1e-6);
    if report.seam_gap_bound_mm > gate
        || (order >= 1 && report.first_jet_bound > gate)
        || (order >= 2 && report.second_jet_bound > gate)
    {
        return Err(invalid(
            "polynomial matching could not meet its roundoff jet bound",
        ));
    }
    let old = net(&normalize(&original, min, width)?)?;
    for (a, b) in control.iter().flatten().zip(old.iter().flatten()) {
        report.source_displacement_bound_mm =
            report.source_displacement_bound_mm.max((*a - *b).length());
    }
    let uv = ordered_uv_loops(
        projected
            .into_iter()
            .map(|l| {
                l.into_iter()
                    .map(|p| [(p[0] - min[0]) / width[0], (p[1] - min[1]) / width[1]])
                    .collect()
            })
            .collect(),
    )?;
    let mut curves = Vec::new();
    for ring in &uv {
        for (a, b) in ring
            .iter()
            .zip(ring.iter().cycle().skip(1))
            .take(ring.len())
        {
            curves.push(edge_curve(&native_power, *a, *b)?);
        }
    }
    report.boundary_edges = curves.len();
    let mut knots = vec![0.; 7];
    knots.extend([1.; 7]);
    let points = control
        .into_iter()
        .map(|r| r.into_iter().map(CPoint::from_point).collect())
        .collect();
    let surface: Surface =
        BSplineSurface::from_gw_nurb(&NurbSurface::new(points, 6, 6, knots.clone(), knots)?).into();
    Ok((trimmed_sheet(k, surface, &uv, 1e-6, Some(curves))?, report))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn patch(source: bool, hole: bool) -> PatchInput {
        let mut points = Vec::new();
        for i in 0usize..4 {
            let mut row = Vec::new();
            for j in 0usize..4 {
                let u = i as f64 / 3.;
                let v = j as f64 / 3.;
                let uu = (i * (i.saturating_sub(1))) as f64 / 6.;
                let vv = (j * (j.saturating_sub(1))) as f64 / 6.;
                let z = if source {
                    0.2 + 0.1 * u + 0.03 * uu + 0.04 * vv
                } else {
                    0.01 * uu + 0.02 * vv
                };
                row.push([u, v, z]);
            }
            points.push(row);
        }
        let outer = if source {
            vec![[0., 0.], [1., 0.], [1., 1.]]
        } else {
            vec![[0., 0.], [1., 1.], [0., 1.]]
        };
        let mut rings = vec![outer];
        if hole {
            rings.push(vec![[0.7, 0.1], [0.7, 0.2], [0.9, 0.2], [0.9, 0.1]]);
        }
        let mut index = 0;
        let loops = rings
            .into_iter()
            .map(|r| {
                r.iter()
                    .zip(r.iter().cycle().skip(1))
                    .take(r.len())
                    .map(|(a, b)| {
                        let edge = TrimSegment {
                            edge_index: index,
                            points: vec![*a, *b],
                            knots: vec![0., 0., 1., 1.],
                            interval: [0., 1.],
                            forward: true,
                        };
                        index += 1;
                        edge
                    })
                    .collect()
            })
            .collect();
        PatchInput {
            points,
            u_knots: vec![0., 0., 0., 0., 1., 1., 1., 1.],
            v_knots: vec![0., 0., 0., 0., 1., 1., 1., 1.],
            loops,
        }
    }
    #[test]
    fn oblique_trim_matches_all_orders_and_keeps_hole() {
        let source = patch(true, true);
        let target = patch(false, false);
        let target_before = format!("{target:?}");
        for order in [MatchOrder::G0, MatchOrder::G1, MatchOrder::G2] {
            let (mut body, r) = match_trimmed(
                &mut Kernel::default(),
                &source,
                2,
                &target,
                0,
                MatchOptions {
                    order,
                    ..MatchOptions::default()
                },
            )
            .unwrap();
            assert_eq!(body.faces().len(), 1);
            assert_eq!(r.boundary_loops, 2);
            assert_eq!(r.boundary_edges, 7);
            assert!(r.seam_gap_bound_mm < 1e-10);
            assert!(r.max_gap_mm < 1e-10);
            if order != MatchOrder::G0 {
                assert!(r.first_jet_bound < 1e-10);
                assert!(r.max_normal_angle_deg < 1e-7);
            }
            if order == MatchOrder::G2 {
                assert!(r.second_jet_bound < 1e-10);
                assert!(r.max_curvature_jump_per_mm < 1e-8);
            }
            let mut notify = crate::topology::brep::p2::ArenaNotifyState::new(&body);
            let env = notify.env(None);
            let check = crate::topology::brep::validation::check_brep_with_keys(
                &mut Kernel::default(),
                &mut body,
                crate::topology::brep::validation::CheckLevel::Level1,
                &|e| (env.key_of)(e),
            )
            .unwrap();
            assert!(check.is_valid(), "{:?}", check.lines());
        }
        assert_eq!(format!("{target:?}"), target_before);
    }
    #[test]
    fn exact_trim_restrictions_stay_on_polynomial() {
        let source = patch(true, false);
        let p = composed(&source, [0.3, 0.1], [0.6, 0.7], [-0.7, 0.6]).unwrap();
        let curve = edge_curve(&p, [0.2, 0.1], [0.8, 0.9]).unwrap();
        let k = Kernel::default();
        for i in 0..21 {
            let t = i as f64 / 20.;
            let expected = p.eval(0.2 + 0.6 * t, 0.1 + 0.8 * t, 0, 0).unwrap();
            let got = curve.evaluate(&k, t, 0, true, false).unwrap()[0];
            assert!((got - expected).length() < 1e-10);
        }
    }
    #[test]
    fn bad_scale_and_unsupported_patch_are_refused() {
        let source = patch(true, false);
        let target = patch(false, false);
        assert!(match_trimmed(
            &mut Kernel::default(),
            &source,
            2,
            &target,
            0,
            MatchOptions {
                cross_scale: 0.,
                ..MatchOptions::default()
            }
        )
        .is_err());
        let mut bad = source.clone();
        bad.points.pop();
        assert!(match_trimmed(
            &mut Kernel::default(),
            &bad,
            2,
            &target,
            0,
            MatchOptions::default()
        )
        .is_err());
    }
    #[test]
    fn curvature_matching_respects_nonunit_cross_reparameterization() {
        let source = patch(true, false);
        let target = patch(false, false);
        let (_, r) = match_trimmed(
            &mut Kernel::default(),
            &source,
            2,
            &target,
            0,
            MatchOptions {
                cross_scale: 0.73,
                ..MatchOptions::default()
            },
        )
        .unwrap();
        assert!(r.max_normal_angle_deg < 1e-7);
        assert!(r.max_curvature_jump_per_mm < 1e-8);
        assert!(r.second_jet_bound < 1e-10);
        let (_, g1) = match_trimmed(
            &mut Kernel::default(),
            &source,
            2,
            &target,
            0,
            MatchOptions {
                order: MatchOrder::G1,
                ..MatchOptions::default()
            },
        )
        .unwrap();
        assert!(
            g1.max_curvature_jump_per_mm > 1e-3,
            "G1 should not falsely report G2"
        );
        let (_, g0) = match_trimmed(
            &mut Kernel::default(),
            &source,
            2,
            &target,
            0,
            MatchOptions {
                order: MatchOrder::G0,
                ..MatchOptions::default()
            },
        )
        .unwrap();
        assert!(
            g0.max_normal_angle_deg > 0.1,
            "G0 should not falsely report G1"
        );
    }
    #[test]
    fn elevated_nonuniform_straight_pcurves_match_without_flattening_curves() {
        let mut source = patch(true, true);
        let target = patch(false, false);
        for edge in source.loops.iter_mut().flatten() {
            let a = edge.points[0];
            let b = edge.points[1];
            edge.points = [0., 0.05, 0.2, 0.5, 0.7, 0.85, 1.]
                .into_iter()
                .map(|t| [a[0] * (1. - t) + b[0] * t, a[1] * (1. - t) + b[1] * t])
                .collect();
            edge.knots = vec![0., 0., 0., 0., 0.5, 0.5, 0.5, 1., 1., 1., 1.];
        }
        let (_, report) = match_trimmed(
            &mut Kernel::default(),
            &source,
            2,
            &target,
            0,
            MatchOptions::default(),
        )
        .unwrap();
        assert!(report.seam_gap_bound_mm < 1e-10);
        assert_eq!(report.boundary_loops, 2);
        source.loops[0][0].points[2][1] += 1e-4;
        assert!(
            match_trimmed(
                &mut Kernel::default(),
                &source,
                2,
                &target,
                0,
                MatchOptions::default()
            )
            .is_err(),
            "genuinely curved pcurves must not be silently flattened"
        );
    }
}
