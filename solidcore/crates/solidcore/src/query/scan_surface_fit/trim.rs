//! Native one-face sheet topology from verified outer and hole UV polygons.
//! Original construction, bounded to4096 boundary segments; no recursion.
use super::invalid;
use crate::curve::curve::Curve;
use crate::geom::extent2d::Extent2d;
use crate::geom::tol::USE_DEFAULT;
use crate::nurbs::b_spline_curve::BSplineCurve;
use crate::nurbs::b_spline_surface_n_lib::p2::OrientType;
use crate::surface::surface::Surface;
use crate::topology::brep::p2::ArenaNotifyState;
use crate::topology::brep::p3::new_brep;
use crate::topology::brep::p7::{FaceCurvesInput, make_face_with_curves};
use crate::{Brep, EntityRef, Kernel, Result, Vec3};

fn area(ring: &[[f64; 2]]) -> f64 {
    ring.iter()
        .zip(ring.iter().cycle().skip(1))
        .take(ring.len())
        .map(|(a, b)| a[0] * b[1] - a[1] * b[0])
        .sum::<f64>()
        * 0.5
}
fn inside(p: [f64; 2], ring: &[[f64; 2]]) -> bool {
    let mut yes = false;
    for (a, b) in ring
        .iter()
        .zip(ring.iter().cycle().skip(1))
        .take(ring.len())
    {
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            yes = !yes;
        }
    }
    yes
}
fn cross(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}
fn intersects(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2], eps: f64) -> bool {
    let between = |a: [f64; 2], b: [f64; 2], p: [f64; 2]| {
        p[0] >= a[0].min(b[0]) - eps
            && p[0] <= a[0].max(b[0]) + eps
            && p[1] >= a[1].min(b[1]) - eps
            && p[1] <= a[1].max(b[1]) + eps
    };
    let (x, y, z, w) = (
        cross(a, b, c),
        cross(a, b, d),
        cross(c, d, a),
        cross(c, d, b),
    );
    (x * y < 0. && z * w < 0.)
        || (x.abs() <= eps && between(a, b, c))
        || (y.abs() <= eps && between(a, b, d))
        || (z.abs() <= eps && between(c, d, a))
        || (w.abs() <= eps && between(c, d, b))
}
// Remove redundant subdivisions only at floating-point arithmetic scale. This is not
// tolerance-driven boundary smoothing: any resolved UV curvature is retained.
fn remove_collinear_vertices(ring: &mut Vec<[f64; 2]>) {
    let budget = ring.len();
    for _ in 0..budget {
        if ring.len() <= 3 {
            break;
        }
        let mut removable = None;
        for i in 0..ring.len() {
            let Some(&a) = ring.get((i + ring.len() - 1) % ring.len()) else {
                continue;
            };
            let Some(&b) = ring.get(i) else { continue };
            let Some(&c) = ring.get((i + 1) % ring.len()) else {
                continue;
            };
            let ab = [b[0] - a[0], b[1] - a[1]];
            let bc = [c[0] - b[0], c[1] - b[1]];
            let forward = ab[0] * bc[0] + ab[1] * bc[1];
            let scale = a
                .into_iter()
                .chain(b)
                .chain(c)
                .map(f64::abs)
                .fold(0., f64::max);
            let roundoff = 4.
                * f64::EPSILON
                * ((ab[0] * bc[1]).abs()
                    + (ab[1] * bc[0]).abs()
                    + scale * (ab[0].abs() + ab[1].abs() + bc[0].abs() + bc[1].abs()));
            if forward > 0. && (ab[0] * bc[1] - ab[1] * bc[0]).abs() <= roundoff {
                removable = Some(i);
                break;
            }
        }
        if let Some(i) = removable {
            ring.remove(i);
        } else {
            break;
        }
    }
}
/// Sort outer first and holes next, and orient them CCW/CW without changing their geometry.
/// Validate simple disjoint loops, order the outer loop first, and orient holes oppositely.
pub fn ordered_uv_loops(mut loops: Vec<Vec<[f64; 2]>>) -> Result<Vec<Vec<[f64; 2]>>> {
    if loops.is_empty() || loops.iter().map(Vec::len).sum::<usize>() > 4096 {
        return Err(invalid("surface boundary must contain1 to4096 edges"));
    }
    let scale = loops
        .iter()
        .flatten()
        .flat_map(|p| p.iter())
        .map(|v| v.abs())
        .fold(1., f64::max);
    let eps = scale * scale * 1e-12;
    for ring in &loops {
        if ring.len() < 3
            || !ring.iter().flatten().all(|v| v.is_finite())
            || area(ring).abs() <= eps
        {
            return Err(invalid("projected boundary is collapsed or nonfinite"));
        }
    }
    for ring in &mut loops {
        remove_collinear_vertices(ring);
    }
    let segments: Vec<_> = loops
        .iter()
        .enumerate()
        .flat_map(|(li, r)| {
            r.iter()
                .copied()
                .zip(r.iter().copied().cycle().skip(1))
                .take(r.len())
                .enumerate()
                .map(move |(i, (a, b))| (li, i, r.len(), a, b))
        })
        .collect();
    for (i, a) in segments.iter().enumerate() {
        for b in segments.iter().skip(i + 1) {
            if a.0 == b.0
                && (a.1 + 1 == b.1
                    || b.1 + 1 == a.1
                    || (a.1 == 0 && b.1 + 1 == a.2)
                    || (b.1 == 0 && a.1 + 1 == a.2))
            {
                continue;
            }
            if intersects(a.3, a.4, b.3, b.4, eps) {
                return Err(invalid(
                    "projected boundary loops intersect or touch; split the region or lower boundary tolerance",
                ));
            }
        }
    }
    loops.sort_by(|a, b| area(b).abs().total_cmp(&area(a).abs()));
    let outer = loops
        .first()
        .ok_or_else(|| invalid("missing outer boundary"))?;
    for (i, hole) in loops.iter().enumerate().skip(1) {
        let p = *hole.first().ok_or_else(|| invalid("empty hole"))?;
        if !inside(p, outer) {
            return Err(invalid(
                "selected region projects to disconnected outer boundaries",
            ));
        }
        for (j, other) in loops.iter().enumerate().skip(1) {
            if i != j && inside(p, other) {
                return Err(invalid(
                    "nested projected boundary holes are not supported as one surface patch",
                ));
            }
        }
    }
    for (i, ring) in loops.iter_mut().enumerate() {
        if (area(ring) > 0.) != (i == 0) {
            ring.reverse();
        }
    }
    Ok(loops)
}
/// Curves supplied by the caller must follow ordered_uv_loops exactly. Native UV curves
/// are retained on every edge; a freeform caller supplies exact restriction curves.
/// Construct one native open face from ordered UV loops and optional exact boundary curves.
pub fn trimmed_sheet(
    k: &mut Kernel,
    surface: Surface,
    loops: &[Vec<[f64; 2]>],
    tolerance: f64,
    exact_edges: Option<Vec<Curve>>,
) -> Result<Brep> {
    if !tolerance.is_finite() || tolerance <= 0. {
        return Err(invalid("construction tolerance must be positive"));
    }
    let count = loops.iter().map(Vec::len).sum::<usize>();
    if count == 0 || count > 4096 {
        return Err(invalid("surface boundary exceeds its construction budget"));
    }
    if exact_edges.as_ref().is_some_and(|v| v.len() != count) {
        return Err(invalid(
            "exact boundary curve count does not match its UV trims",
        ));
    }
    let mut uv_curves = Vec::new();
    let mut minimum = [f64::INFINITY; 2];
    let mut maximum = [f64::NEG_INFINITY; 2];
    for ring in loops {
        for (a, b) in ring
            .iter()
            .zip(ring.iter().cycle().skip(1))
            .take(ring.len())
        {
            for (axis, &v) in a.iter().enumerate() {
                if let Some(m) = minimum.get_mut(axis) {
                    *m = m.min(v);
                }
                if let Some(m) = maximum.get_mut(axis) {
                    *m = m.max(v);
                }
            }
            uv_curves.push(Curve::from(BSplineCurve::create_line_segment(
                2,
                Vec3::new(a[0], a[1], 0.),
                Vec3::new(b[0], b[1], 0.),
            )?));
        }
    }
    let domain = Extent2d::from_bounds(minimum[0], minimum[1], maximum[0], maximum[1]);
    let mut body = new_brep(k, USE_DEFAULT, USE_DEFAULT)?;
    body.set_tolerance(tolerance);
    let mut state = ArenaNotifyState::new(&body);
    let mut env = state.env(None);
    let key_of = env.key_of;
    let this_face = |f| key_of(EntityRef::Face(f));
    make_face_with_curves(
        k,
        &mut body,
        None,
        FaceCurvesInput {
            loops: loops.iter().map(Vec::len).collect(),
            curves_3d: exact_edges.map(|v| v.into_iter().map(Some).collect()),
            uv_curves: Some(uv_curves),
            orientations: vec![OrientType::Same; count],
            loop_points: Vec::new(),
        },
        surface,
        &domain,
        OrientType::Same,
        Some(false),
        &this_face,
        &mut env,
    )?;
    body.refresh_cached_topology();
    Ok(body)
}
