//! Exact XT geometry carriers. Layout follows the public UGS XT format reference
//! and the Apache-2.0 base schema in cadmpeg-parasolid; no tessellation is used.
//!
//! Spatial coordinates are metres; NURBS knot values and weights are unchanged.
//! Analytic carriers have XT's natural parameterization: line parameters become
//! metric distance, plane axes become orthonormal, and torus v loses tube_start.
//! Cone parameter maps are (u,v)->(-u,v/1000) for positive source half-angle,
//! and (u,v)->(u,-v/1000) for negative angle. Its negative carrier sense restores
//! the source surface normal. Callers must reconstruct trims from 3D topology,
//! or map their pcurves; source analytic UVs cannot be copied verbatim.
use super::{Field as F, Writer, unsupported};
use crate::Result;
use solidcore_record::brep::{Curve, Frame, Surface};

fn mm(p: [f64; 3]) -> [f64; 3] {
    p.map(|x| x / 1000.)
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn unit(a: [f64; 3]) -> Result<[f64; 3]> {
    let scale = a.into_iter().map(f64::abs).fold(0., f64::max);
    if !scale.is_finite() || scale == 0. || a.iter().any(|v| !v.is_finite()) {
        return Err(unsupported("geometry has a nonfinite or zero direction"));
    }
    let a = a.map(|v| v / scale);
    let length = a.iter().map(|v| v * v).sum::<f64>().sqrt();
    Ok(a.map(|v| v / length))
}
fn finite(values: impl IntoIterator<Item = f64>) -> Result<()> {
    if values.into_iter().all(f64::is_finite) { Ok(()) } else { Err(unsupported("geometry contains nonfinite coordinates")) }
}
fn positive(v: f64) -> Result<f64> {
    if v.is_finite() && v > 0. { Ok(v / 1000.) } else { Err(unsupported("geometry has a nonpositive or nonfinite radius")) }
}
fn frame(f: &Frame) -> Result<([f64; 3], [f64; 3])> {
    finite(f.origin)?;
    let x = unit(f.x_axis)?;
    let y = unit(f.y_axis)?;
    if x.iter().zip(y).map(|(a, b)| a * b).sum::<f64>().abs() > 1e-10 {
        return Err(unsupported("analytic geometry frame is not orthogonal"));
    }
    Ok((unit(cross(x, y))?, x))
}
fn header(id: usize, owner: usize, sense: char) -> Vec<F> {
    vec![F::I(id + 1), F::P(0), F::P(owner), F::P(0), F::P(0), F::P(0), F::C(sense)]
}
fn analytic(w: &mut Writer, kind: u16, id: usize, owner: usize, sense: char, values: Vec<F>) -> Result<()> {
    let mut fields = header(id, owner, sense);
    fields.extend(values);
    w.node(kind, id, fields)
}

pub(super) fn curve(w: &mut Writer, id: usize, owner: usize, curve: &Curve) -> Result<()> {
    match curve {
        Curve::Line { start, end } => {
            finite(start.iter().chain(end).copied())?;
            analytic(w, 30, id, owner, '+', vec![F::V(mm(*start)), F::V(unit(sub(*end, *start))?)])
        }
        Curve::Circle { frame: f, radius } => {
            let (z, x) = frame(f)?;
            analytic(w, 31, id, owner, '+', vec![F::V(mm(f.origin)), F::V(z), F::V(x), F::R(positive(*radius)?)])
        }
        Curve::Ellipse { frame: f, radii } => {
            let (z, mut x) = frame(f)?;
            let mut r = [positive(radii[0])?, positive(radii[1])?];
            // A swapped ellipse has t_xt=t_source-pi/2, with unchanged tangent sense.
            if r[1] > r[0] {
                r.swap(0, 1);
                x = unit(f.y_axis)?;
            }
            analytic(w, 32, id, owner, '+', vec![F::V(mm(f.origin)), F::V(z), F::V(x), F::R(r[0]), F::R(r[1])])
        }
        Curve::Polyline { points } => {
            if points.len() < 2 {
                return Err(unsupported("polyline has fewer than two points"));
            }
            let mut knots = Vec::with_capacity(points.len() + 2);
            knots.push(0.);
            knots.extend((0..points.len()).map(|i| i as f64));
            knots.push((points.len() - 1) as f64);
            curve_nurbs(w, id, owner, &knots, points.iter().map(|p| p.to_vec()).collect(), false)
        }
        Curve::Spline { knots, points } => curve_nurbs(w, id, owner, knots, points.iter().map(|p| p.to_vec()).collect(), false),
        Curve::RationalSpline { knots, homogeneous } => curve_nurbs(w, id, owner, knots, homogeneous.iter().map(|p| p.to_vec()).collect(), true),
        Curve::SurfaceIntersection { .. } => Err(unsupported("exact surface-intersection curve carrier is not implemented")),
    }
}

pub(super) fn surface(w: &mut Writer, id: usize, owner: usize, surface: &Surface) -> Result<()> {
    match surface {
        Surface::Plane { origin, u_axis, v_axis } => plane(w, id, owner, *origin, *u_axis, *v_axis),
        Surface::PlaneThrough { origin, u_point, v_point } => plane(w, id, owner, *origin, sub(*u_point, *origin), sub(*v_point, *origin)),
        Surface::Cylinder { frame: f, radius } => {
            let (z, x) = frame(f)?;
            analytic(w, 51, id, owner, '+', vec![F::V(mm(f.origin)), F::V(z), F::R(positive(*radius)?), F::V(x)])
        }
        Surface::Cone { frame: f, radius_at_origin, half_angle } => {
            let (mut z, x) = frame(f)?;
            if !radius_at_origin.is_finite()
                || *radius_at_origin < 0.
                || !half_angle.is_finite()
                || half_angle.abs() < 1e-12
                || half_angle.abs() >= std::f64::consts::FRAC_PI_2
            {
                return Err(unsupported("cone radius or half-angle is outside the exact analytic domain"));
            }
            if *half_angle > 0. {
                z = z.map(|v| -v);
            }
            let a = half_angle.abs();
            analytic(w, 52, id, owner, '-', vec![F::V(mm(f.origin)), F::V(z), F::R(radius_at_origin / 1000.), F::R(a.sin()), F::R(a.cos()), F::V(x)])
        }
        Surface::Sphere { frame: f, radius } => {
            let (z, x) = frame(f)?;
            analytic(w, 53, id, owner, '+', vec![F::V(mm(f.origin)), F::R(positive(*radius)?), F::V(z), F::V(x)])
        }
        Surface::Torus { frame: f, major_radius, minor_radius, tube_start } => {
            let (z, x) = frame(f)?;
            finite([*tube_start])?;
            analytic(
                w,
                54,
                id,
                owner,
                '+',
                vec![F::V(mm(f.origin)), F::V(z), F::R(positive(*major_radius)?), F::R(positive(*minor_radius)?), F::V(x)],
            )
        }
        Surface::Spline { u_knots, v_knots, points } => {
            surface_nurbs(w, id, owner, u_knots, v_knots, points.iter().map(|row| row.iter().map(|p| p.to_vec()).collect()).collect(), false)
        }
        Surface::RationalSpline { u_knots, v_knots, homogeneous } => {
            surface_nurbs(w, id, owner, u_knots, v_knots, homogeneous.iter().map(|row| row.iter().map(|p| p.to_vec()).collect()).collect(), true)
        }
        Surface::BallPatch { .. } => Err(unsupported("ball patch requires its exact NURBS surface form")),
        Surface::PlacedRevolution { .. } => Err(unsupported("placed revolution requires its exact NURBS surface form")),
    }
}
fn plane(w: &mut Writer, id: usize, owner: usize, origin: [f64; 3], u: [f64; 3], v: [f64; 3]) -> Result<()> {
    finite(origin)?;
    analytic(w, 50, id, owner, '+', vec![F::V(mm(origin)), F::V(unit(cross(unit(u)?, unit(v)?))?), F::V(unit(u)?)])
}

struct Knots {
    degree: usize,
    values: Vec<f64>,
    multiplicities: Vec<usize>,
}
fn knots(knots: &[f64], count: usize) -> Result<Knots> {
    let degree = knots.len().checked_sub(count).and_then(|n| n.checked_sub(1)).ok_or_else(|| unsupported("NURBS knot/control counts disagree"))?;
    if degree == 0
        || degree >= count
        || degree > 32767
        || knots.iter().any(|v| !v.is_finite())
        || knots.windows(2).any(|p| p[0] > p[1])
        || knots[degree] >= knots[count]
    {
        return Err(unsupported("NURBS has invalid degree, knot order or parameter domain"));
    }
    let mut values = Vec::new();
    let mut multiplicities: Vec<usize> = Vec::new();
    for &k in knots {
        if values.last() == Some(&k) {
            if let Some(n) = multiplicities.last_mut() {
                *n += 1;
            }
        } else {
            values.push(k);
            multiplicities.push(1);
        }
    }
    if multiplicities.iter().any(|m| *m > degree + 1) {
        return Err(unsupported("NURBS knot multiplicity exceeds its order"));
    }
    Ok(Knots { degree, values, multiplicities })
}
fn poles(points: &[Vec<f64>], rational: bool) -> Result<Vec<F>> {
    let dimension = if rational { 4 } else { 3 };
    let mut out = Vec::with_capacity(points.len() * dimension);
    for point in points {
        if point.len() != dimension || point.iter().any(|v| !v.is_finite()) || (rational && point[3] <= 0.) {
            return Err(unsupported("NURBS has malformed control points or nonpositive weights"));
        }
        out.extend(point.iter().enumerate().map(|(i, v)| F::R(if i < 3 { *v / 1000. } else { *v })));
    }
    Ok(out)
}
fn knot_arrays(w: &mut Writer, k: &Knots) -> Result<(usize, usize)> {
    let mult = w.reserve();
    let values = w.reserve();
    w.array(127, mult, k.multiplicities.iter().copied().map(F::I).collect())?;
    w.array(128, values, k.values.iter().copied().map(F::R).collect())?;
    Ok((mult, values))
}
fn same_point(a: &[f64], b: &[f64], rational: bool) -> bool {
    if rational { (0..3).all(|i| a[i] / a[3] == b[i] / b[3]) } else { a == b }
}
fn curve_nurbs(w: &mut Writer, id: usize, owner: usize, kv: &[f64], points: Vec<Vec<f64>>, rational: bool) -> Result<()> {
    let k = knots(kv, points.len())?;
    let values = poles(&points, rational)?;
    let closed = points.first().zip(points.last()).is_some_and(|(a, b)| same_point(a, b, rational));
    let data = w.reserve();
    let vertices = w.reserve();
    let auxiliary = w.reserve();
    let (mult, knots) = knot_arrays(w, &k)?;
    w.array(45, vertices, values)?;
    w.node(135, auxiliary, vec![F::I(1), F::P(0)])?;
    w.node(
        136,
        data,
        vec![
            F::I(k.degree),
            F::I(points.len()),
            F::I(if rational { 4 } else { 3 }),
            F::I(k.values.len()),
            F::I(1),
            F::L(false),
            F::L(closed),
            F::L(rational),
            F::I(1),
            F::P(vertices),
            F::P(mult),
            F::P(knots),
        ],
    )?;
    analytic(w, 134, id, owner, '+', vec![F::P(data), F::P(auxiliary)])
}
fn surface_nurbs(w: &mut Writer, id: usize, owner: usize, uk: &[f64], vk: &[f64], points: Vec<Vec<Vec<f64>>>, rational: bool) -> Result<()> {
    let nu = points.len();
    let nv = points.first().map_or(0, Vec::len);
    if nv == 0 || points.iter().any(|row| row.len() != nv) {
        return Err(unsupported("NURBS surface control net is empty or ragged"));
    }
    let u = knots(uk, nu)?;
    let v = knots(vk, nv)?;
    let flat: Vec<_> = points.iter().flatten().cloned().collect();
    let values = poles(&flat, rational)?;
    let uclosed = (0..nv).all(|j| same_point(&points[0][j], &points[nu - 1][j], rational));
    let vclosed = points.iter().all(|row| same_point(&row[0], &row[nv - 1], rational));
    let data = w.reserve();
    let vertices = w.reserve();
    let (um, us) = knot_arrays(w, &u)?;
    let (vm, vs) = knot_arrays(w, &v)?;
    w.array(45, vertices, values)?;
    w.node(
        126,
        data,
        vec![
            F::L(false),
            F::L(false),
            F::I(u.degree),
            F::I(v.degree),
            F::I(nu),
            F::I(nv),
            F::I(1),
            F::I(1),
            F::I(u.values.len()),
            F::I(v.values.len()),
            F::L(rational),
            F::L(uclosed),
            F::L(vclosed),
            F::I(1),
            F::I(if rational { 4 } else { 3 }),
            F::P(vertices),
            F::P(um),
            F::P(vm),
            F::P(us),
            F::P(vs),
        ],
    )?;
    analytic(w, 124, id, owner, '+', vec![F::P(data), F::P(0)])
}

/// Encode a bounded line as an exact degree-one spline without changing t.
/// This avoids unit-distance reparameterization when pcurves use the source t.
pub(super) fn line_domain(w: &mut Writer, id: usize, owner: usize, start: [f64; 3], end: [f64; 3], interval: [f64; 2]) -> Result<()> {
    finite(start.into_iter().chain(end).chain(interval))?;
    if interval[0] >= interval[1] {
        return Err(unsupported("line has an empty parameter interval"));
    }
    let direction = sub(end, start);
    unit(direction)?;
    let points = interval.into_iter().map(|t| (0..3).map(|i| start[i] + t * direction[i]).collect()).collect();
    curve_nurbs(w, id, owner, &[interval[0], interval[0], interval[1], interval[1]], points, false)
}

/// Parameter-space B-curve. Only its coordinates are mapped; its knot parameter
/// remains the spatial edge parameter. Rational offsets are multiplied by w.
pub(super) fn pcurve(w: &mut Writer, id: usize, owner: usize, c: &solidcore_record::brep::Curve2, scale: [f64; 2], offset: [f64; 2]) -> Result<()> {
    use solidcore_record::brep::Curve2;
    finite(scale.into_iter().chain(offset))?;
    if scale.contains(&0.) {
        return Err(unsupported("pcurve coordinate map is singular"));
    }
    let (kv, points, rational) = match c {
        Curve2::Spline { knots, points } => {
            (knots, points.iter().map(|p| vec![p[0] * scale[0] + offset[0], p[1] * scale[1] + offset[1]]).collect::<Vec<_>>(), false)
        }
        Curve2::RationalSpline { knots, homogeneous } => (
            knots,
            homogeneous.iter().map(|p| vec![p[0] * scale[0] + offset[0] * p[2], p[1] * scale[1] + offset[1] * p[2], p[2]]).collect::<Vec<_>>(),
            true,
        ),
    };
    let k = knots(kv, points.len())?;
    if points.iter().flatten().any(|v| !v.is_finite()) || (rational && points.iter().any(|p| p[2] <= 0.)) {
        return Err(unsupported("pcurve has nonfinite coordinates or nonpositive weights"));
    }
    let closed = points.first().zip(points.last()).is_some_and(|(a, b)| if rational { (0..2).all(|i| a[i] / a[2] == b[i] / b[2]) } else { a == b });
    let data = w.reserve();
    let vertices = w.reserve();
    let auxiliary = w.reserve();
    let (mult, ks) = knot_arrays(w, &k)?;
    w.array(45, vertices, points.iter().flatten().copied().map(F::R).collect())?;
    w.node(135, auxiliary, vec![F::I(1), F::P(0)])?;
    w.node(
        136,
        data,
        vec![
            F::I(k.degree),
            F::I(points.len()),
            F::I(if rational { 3 } else { 2 }),
            F::I(k.values.len()),
            F::I(1),
            F::L(false),
            F::L(closed),
            F::L(rational),
            F::I(1),
            F::P(vertices),
            F::P(mult),
            F::P(ks),
        ],
    )?;
    analytic(w, 134, id, owner, '+', vec![F::P(data), F::P(auxiliary)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knots_keep_nonunit_domain_and_exact_multiplicities() -> Result<()> {
        let k = knots(&[2., 2., 2., 3., 5., 5., 5.], 4)?;
        assert_eq!(k.degree, 2);
        assert_eq!(k.values, [2., 3., 5.]);
        assert_eq!(k.multiplicities, [3, 1, 3]);
        Ok(())
    }

    #[test]
    fn homogeneous_weights_are_not_scaled_with_spatial_coordinates() -> Result<()> {
        let values = poles(&[vec![100., 200., 300., 2.]], true)?;
        for (actual, expected) in values.iter().zip([0.1, 0.2, 0.3, 2.]) {
            assert!(matches!(actual, F::R(v) if *v == expected));
        }
        assert!(poles(&[vec![1., 2., 3., 0.]], true).is_err());
        Ok(())
    }

    #[test]
    fn invalid_geometry_fails_without_approximation() {
        assert!(knots(&[0., 1., 0., 1.], 2).is_err());
        assert!(knots(&[0., 0., 0., 0.], 2).is_err());
        assert!(unit([0.; 3]).is_err());
        assert!(unit([f64::NAN, 1., 0.]).is_err());
        assert!(positive(-1.).is_err());
    }
}
