//! Plane and rotational primitive fitting, and boundary projection onto native surfaces.
//! Original implementation: covariance/normal-plane initialization and bounded geometric
//! least-squares refinement. No recursion; at most30 refinement iterations over15000 samples.
use super::{Kind, Options, PreparedRegion, Report, invalid, ordered_uv_loops, trimmed_sheet};
use crate::geom::{
    axis2_placement::Axis2Placement, extent2d::Extent2d, matrix::Matrix, vector2d::Vec2,
};
use crate::surface::{cone::Cone, plane::Plane, surface::Surface};
use crate::{Brep, Kernel, Result, Vec3};
use std::collections::BTreeMap;

fn solve(a: [[f64; 3]; 3], b: [f64; 3]) -> Result<Vec3> {
    let matrix = Matrix::from_row_major(3, 3, a.into_iter().flatten().collect())?;
    let answer = matrix.solve_linear_system(&b)?;
    if answer.near_singular {
        return Err(invalid(
            "selected region does not resolve a stable primitive",
        ));
    }
    let get = |i| {
        answer
            .x
            .get(i)
            .copied()
            .ok_or_else(|| invalid("incomplete least-squares solution"))
    };
    Ok(Vec3::new(get(0)?, get(1)?, get(2)?))
}
fn basis(axis: Vec3) -> Result<(Vec3, Vec3)> {
    let auxiliary = if axis.x.abs() < 0.8 { Vec3::X } else { Vec3::Y };
    let x = (auxiliary - axis * axis.dot(auxiliary)).unitize()?;
    Ok((x, axis.cross(x).unitize()?))
}
fn normal_axis(region: &PreparedRegion) -> Result<Vec3> {
    let mut total = 0.;
    let mut mean = Vec3::ZERO;
    for &(n, _, w) in &region.face_normals {
        mean += n * w;
        total += w;
    }
    if total <= 0. {
        return Err(invalid("region has no usable normals"));
    }
    mean = mean / total;
    let mut m = Matrix::new(3, 3);
    for &(n, _, w) in &region.face_normals {
        let d = n - mean;
        for i in 0..3 {
            for j in i..3 {
                let value = m.at(i, j)? + w * d.component(i) * d.component(j);
                m.set_at(i, j, value)?;
                m.set_at(j, i, value)?;
            }
        }
    }
    let (values, axes) = m.compute_3x3_eigen_vectors()?;
    if values[1] <= total * 1e-7 {
        return Err(invalid(
            "choose a wider curved region; its normals do not resolve a cylinder or cone axis",
        ));
    }
    let mut axis = axes[0].unitize()?;
    let dominant = if axis.x.abs() >= axis.y.abs() && axis.x.abs() >= axis.z.abs() {
        axis.x
    } else if axis.y.abs() >= axis.z.abs() {
        axis.y
    } else {
        axis.z
    };
    if dominant < 0. {
        axis = -axis;
    }
    Ok(axis)
}
fn initial(region: &PreparedRegion, kind: Kind) -> Result<[f64; 7]> {
    let mut axis = normal_axis(region)?;
    let (x, y) = basis(axis)?;
    let (origin, size) = if kind == Kind::Cylinder {
        let mut a = [[0.; 3]; 3];
        let mut rhs = [0.; 3];
        for &(p, w) in &region.samples {
            let d = p - region.center;
            let u = d.dot(x);
            let v = d.dot(y);
            let row = [u, v, 1.];
            let q = u * u + v * v;
            for (i, ai) in a.iter_mut().enumerate() {
                let ri = row.get(i).copied().unwrap_or(0.);
                if let Some(b) = rhs.get_mut(i) {
                    *b += w * ri * q;
                }
                for (j, aij) in ai.iter_mut().enumerate() {
                    *aij += w * ri * row.get(j).copied().unwrap_or(0.);
                }
            }
        }
        let c = solve(a, rhs)?;
        let cx = c.x * 0.5;
        let cy = c.y * 0.5;
        let square = c.z + cx * cx + cy * cy;
        if square <= 0. {
            return Err(invalid("cylindrical radius could not be resolved"));
        }
        (region.center + x * cx + y * cy, square.sqrt())
    } else {
        let mut a = [[0.; 3]; 3];
        let mut rhs = [0.; 3];
        for &(n, p, w) in &region.face_normals {
            let d = n.dot(p - region.center);
            for (i, ai) in a.iter_mut().enumerate() {
                if let Some(b) = rhs.get_mut(i) {
                    *b += w * n.component(i) * d;
                }
                for (j, aij) in ai.iter_mut().enumerate() {
                    *aij += w * n.component(i) * n.component(j);
                }
            }
        }
        let apex = region.center + solve(a, rhs)?;
        if (region.center - apex).dot(axis) < 0. {
            axis = -axis;
        }
        let mut hh = 0.;
        let mut hr = 0.;
        for &(p, w) in &region.samples {
            let d = p - apex;
            let h = d.dot(axis);
            let r = (d - axis * h).length();
            hh += w * h * h;
            hr += w * h * r;
        }
        if hh <= 1e-16 {
            return Err(invalid("cone height could not be resolved"));
        }
        let slope = hr / hh;
        if !(1e-4..100.).contains(&slope) {
            return Err(invalid(
                "selected region does not define a stable non-cylindrical cone",
            ));
        }
        (apex, slope)
    };
    Ok([origin.x, origin.y, origin.z, axis.x, axis.y, axis.z, size])
}
fn state(p: &[f64; 7]) -> Option<(Vec3, Vec3, f64)> {
    if !p.iter().all(|v| v.is_finite() && v.abs() < 1e9) || p[6] <= 1e-9 {
        return None;
    }
    Some((
        Vec3::new(p[0], p[1], p[2]),
        Vec3::new(p[3], p[4], p[5]).unitize().ok()?,
        p[6],
    ))
}
fn deviation(point: Vec3, p: &[f64; 7], kind: Kind) -> Option<f64> {
    let (origin, axis, size) = state(p)?;
    let d = point - origin;
    let h = d.dot(axis);
    let r = (d - axis * h).length();
    Some(if kind == Kind::Cylinder {
        r - size
    } else {
        (r - size * h) / (1. + size * size).sqrt()
    })
}
fn refine(region: &PreparedRegion, kind: Kind, mut p: [f64; 7]) -> Result<[f64; 7]> {
    let total = region.samples.iter().map(|v| v.1).sum::<f64>();
    let mut lambda = 1e-6_f64;
    let error = |p: &[f64; 7]| -> Option<f64> {
        let mut sum = 0.;
        for &(point, w) in &region.samples {
            let e = deviation(point, p, kind)?;
            sum += w * e * e / total;
        }
        Some(sum)
    };
    let mut current = error(&p).ok_or_else(|| invalid("invalid primitive initialization"))?;
    for _ in 0..30 {
        let mut a = Matrix::new(7, 7);
        let mut rhs = [0.; 7];
        for &(point, w) in &region.samples {
            let e = deviation(point, &p, kind)
                .ok_or_else(|| invalid("invalid primitive parameters"))?;
            let mut grad = [0.; 7];
            for (i, g) in grad.iter_mut().enumerate() {
                let mut next = p;
                let at = p.get(i).copied().unwrap_or(0.);
                let h = 1e-5_f64.max(at.abs() * 1e-8);
                if let Some(v) = next.get_mut(i) {
                    *v += h;
                }
                *g = (deviation(point, &next, kind)
                    .ok_or_else(|| invalid("primitive derivative is undefined"))?
                    - e)
                    / h;
            }
            let w = w / total;
            for (i, &gi) in grad.iter().enumerate() {
                if let Some(b) = rhs.get_mut(i) {
                    *b -= w * gi * e;
                }
                for (j, &gj) in grad.iter().enumerate() {
                    a.set_at(i, j, a.at(i, j)? + w * gi * gj)?;
                }
            }
        }
        for i in 0..7 {
            a.set_at(i, i, a.at(i, i)? + lambda)?;
        }
        let step = a.solve_linear_system(&rhs)?;
        if step.near_singular {
            break;
        }
        let mut next = p;
        for (v, &d) in next.iter_mut().zip(&step.x) {
            *v += d;
        }
        if let Some(candidate) = error(&next).filter(|v| *v < current) {
            let improvement = current - candidate;
            p = next;
            current = candidate;
            lambda = (lambda * 0.3).max(1e-12);
            if improvement < 1e-14 * current.max(1.) {
                break;
            }
        } else {
            lambda *= 10.;
            if lambda > 1e8 {
                break;
            }
        }
    }
    state(&p).ok_or_else(|| invalid("fitted primitive has invalid dimensions"))?;
    Ok(p)
}
fn report(
    region: &PreparedRegion,
    options: &Options,
    parameters: BTreeMap<String, f64>,
    distance: impl Fn(Vec3) -> f64,
) -> Result<Report> {
    let mut sum = 0.;
    let mut maximum = 0.0_f64;
    let mut count = 0usize;
    for p in region
        .positions
        .iter()
        .copied()
        .chain(region.triangles.iter().filter_map(|t| {
            Some(
                (*region.positions.get(t[0])?
                    + *region.positions.get(t[1])?
                    + *region.positions.get(t[2])?)
                    / 3.,
            )
        }))
    {
        let d = distance(p);
        if !d.is_finite() {
            return Err(invalid("nonfinite fitted surface residual"));
        }
        sum += d * d;
        maximum = maximum.max(d.abs());
        count += 1;
    }
    if count == 0 {
        return Err(invalid("no measured scan samples"));
    }
    Ok(Report{kind:format!("{:?}",options.kind).to_lowercase(),rms_mm:(sum/count as f64).sqrt(),max_mm:maximum,within_tolerance:maximum<=options.tolerance,boundary_loops:region.loops.len(),boundary_vertices:region.loops.iter().map(Vec::len).sum(),control_points_u:0,control_points_v:0,parameters,metric:"Orthogonal distance to the supporting surface, measured at vertices and facet centres; excludes trim-boundary distance.".into(),sampled_points:count,region_triangles:region.triangles.len()})
}
pub(super) fn fit(
    k: &mut Kernel,
    region: &PreparedRegion,
    options: &Options,
) -> Result<(Brep, Report)> {
    let mut parameters = BTreeMap::new();
    if options.kind == Kind::Plane {
        let mut x = region.x;
        if let Some(flow) = options.flow {
            let f = Vec3::from_array(flow);
            x = (f - region.normal * f.dot(region.normal))
                .unitize()
                .map_err(|_| invalid("flow direction must lie along the fitted plane"))?;
        }
        let y = region.normal.cross(x).unitize()?;
        let project = |p: Vec3| {
            let d = p - region.center;
            [d.dot(x), d.dot(y)]
        };
        let loops = ordered_uv_loops(
            region
                .loops
                .iter()
                .map(|l| {
                    l.iter()
                        .map(|i| {
                            region
                                .positions
                                .get(*i)
                                .copied()
                                .map(project)
                                .ok_or_else(|| invalid("missing boundary point"))
                        })
                        .collect()
                })
                .collect::<Result<Vec<Vec<_>>>>()?,
        )?;
        let min = loops
            .iter()
            .flatten()
            .fold([f64::INFINITY; 2], |a, p| [a[0].min(p[0]), a[1].min(p[1])]);
        let max = loops.iter().flatten().fold([f64::NEG_INFINITY; 2], |a, p| {
            [a[0].max(p[0]), a[1].max(p[1])]
        });
        let domain = Extent2d::from_bounds(min[0], min[1], max[0], max[1]);
        let mut plane = Plane::new(region.center, x, y, Vec2::new(1., 1.), &domain);
        plane.make_nurb()?;
        for (key, v) in [
            ("origin_x", region.center.x),
            ("origin_y", region.center.y),
            ("origin_z", region.center.z),
            ("normal_x", region.normal.x),
            ("normal_y", region.normal.y),
            ("normal_z", region.normal.z),
        ] {
            parameters.insert(key.into(), v);
        }
        let mut output = report(region, options, parameters, |p| {
            (p - region.center).dot(region.normal)
        })?;
        output.boundary_vertices = loops.iter().map(Vec::len).sum();
        return Ok((
            trimmed_sheet(k, Surface::from(plane), &loops, 1e-6, None)?,
            output,
        ));
    }
    let fitted = refine(region, options.kind, initial(region, options.kind)?)?;
    let (origin, axis, size) = state(&fitted).ok_or_else(|| invalid("invalid fitted axis"))?;
    let (mut x, mut y) = basis(axis)?;
    let mut angles = Vec::new();
    for p in &region.positions {
        let d = *p - origin;
        angles.push(d.dot(y).atan2(d.dot(x)).rem_euclid(std::f64::consts::TAU));
    }
    angles.sort_by(f64::total_cmp);
    let mut gap = 0.;
    let mut seam = 0.;
    for (i, (a, b)) in angles
        .iter()
        .copied()
        .zip(angles.iter().copied().cycle().skip(1))
        .take(angles.len())
        .enumerate()
    {
        let b = if i + 1 == angles.len() {
            b + std::f64::consts::TAU
        } else {
            b
        };
        if b - a > gap {
            gap = b - a;
            seam = (a + b) * 0.5;
        }
    }
    if gap < 0.02 {
        return Err(invalid(
            "a fitted rotational patch crosses a full wrap; split it at a seam before fitting",
        ));
    }
    x = x * seam.cos() + y * seam.sin();
    y = axis.cross(x).unitize()?;
    let project = |p: Vec3| {
        let d = p - origin;
        let h = d.dot(axis);
        let radius = (d - axis * h).length();
        let h = if options.kind == Kind::Cone {
            h + size * (radius - size * h) / (1. + size * size)
        } else {
            h
        };
        [
            d.dot(y)
                .atan2(d.dot(x))
                .rem_euclid(std::f64::consts::TAU)
                .to_degrees(),
            h,
        ]
    };
    let mut low = f64::INFINITY;
    let mut high = f64::NEG_INFINITY;
    for p in &region.positions {
        let uv = project(*p);
        low = low.min(uv[1]);
        high = high.max(uv[1]);
    }
    if high - low < 1e-7 || options.kind == Kind::Cone && low <= 1e-7 {
        return Err(invalid(
            "selected cone/cylinder patch has no stable height or crosses its apex",
        ));
    }
    let frame = Axis2Placement::new(origin + axis * low, x, y);
    let (bottom, top) = if options.kind == Kind::Cylinder {
        (size, size)
    } else {
        (size * low, size * high)
    };
    let cone = Cone::create_canonical(k, &frame, bottom, top, high - low, false, false)?;
    let loops = ordered_uv_loops(
        region
            .loops
            .iter()
            .map(|l| {
                l.iter()
                    .map(|i| {
                        let p = *region
                            .positions
                            .get(*i)
                            .ok_or_else(|| invalid("missing boundary point"))?;
                        let uv = project(p);
                        let uv =
                            cone.convert_uv_from_step_to_nurbs(k, Vec2::new(uv[0], uv[1] - low))?;
                        Ok([uv.x, uv.y])
                    })
                    .collect()
            })
            .collect::<Result<Vec<Vec<_>>>>()?,
    )?;
    for (key, v) in [
        ("origin_x", origin.x),
        ("origin_y", origin.y),
        ("origin_z", origin.z),
        ("axis_x", axis.x),
        ("axis_y", axis.y),
        ("axis_z", axis.z),
        ("height_mm", high - low),
        ("radius_start_mm", bottom),
        ("radius_end_mm", top),
    ] {
        parameters.insert(key.into(), v);
    }
    if options.kind == Kind::Cone {
        parameters.insert("half_angle_deg".into(), size.atan().to_degrees());
    }
    let mut output = report(region, options, parameters, |p| {
        deviation(p, &fitted, options.kind).unwrap_or(f64::NAN)
    })?;
    output.boundary_vertices = loops.iter().map(Vec::len).sum();
    Ok((
        trimmed_sheet(k, Surface::from(cone), &loops, 1e-6, None)?,
        output,
    ))
}
