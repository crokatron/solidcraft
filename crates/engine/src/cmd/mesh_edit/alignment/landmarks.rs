use super::*;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(in crate::cmd::mesh_edit) struct Landmarks {
    moving: String,
    fixed: String,
    moving_points: [[f64; 3]; 3],
    fixed_points: [[f64; 3]; 3],
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(in crate::cmd::mesh_edit) struct Datum {
    moving: String,
    origin: [f64; 3],
    x_point: [f64; 3],
    xy_point: [f64; 3],
}
#[derive(serde::Serialize, JsonSchema)]
pub(in crate::cmd::mesh_edit) struct ResultData {
    moving: String,
    rms_before_mm: f64,
    rms_after_mm: f64,
    align_report: String,
}
fn triple(raw: [[f64; 3]; 3], id: &str) -> Result<[Vec3; 3]> {
    if !raw.iter().flatten().all(|v| v.is_finite() && v.abs() <= 1e9) {
        return Err(bad(id, "landmarks must be finite world positions within the model bounds"));
    }
    Ok(raw.map(|[x, y, z]| Vec3::new(x, y, z)))
}
/// Columns are the X direction, in-plane Y and their right-handed normal. Reject a
/// nearly collinear triple relative to its longest edge before any source edit.
fn basis([o, a, b]: [Vec3; 3], id: &str) -> Result<Mat> {
    let length = (a - o).len().max((b - o).len()).max((b - a).len());
    let cross = (a - o).cross(b - o);
    if length <= 1e-9 || cross.len() <= 1e-8 * length * length {
        return Err(bad(id, "pick three distinct, noncollinear landmarks"));
    }
    let x = (a - o) / (a - o).len();
    let z = cross / cross.len();
    let y = z.cross(x);
    Ok([[x.x, x.y, x.z, 0.], [y.x, y.y, y.z, 0.], [z.x, z.y, z.z, 0.], [o.x, o.y, o.z, 1.]])
}

fn error(points: [Vec3; 3], targets: [Vec3; 3], m: &Mat) -> f64 {
    (points.into_iter().zip(targets).map(|(a, b)| (apply_point(m, a) - b).len2()).sum::<f64>() / 3.).sqrt()
}

/// Three noncollinear pairs span two planes. Map their oriented normals, then solve
/// the remaining in-plane rotation by 2-D orthogonal Procrustes about each centroid.
fn fit(points: [Vec3; 3], targets: [Vec3; 3], id: &str) -> Result<Mat> {
    let mut a = basis(points, id)?;
    let mut b = basis(targets, id)?;
    for (frame, cloud) in [(&mut a, points), (&mut b, targets)] {
        let center = (cloud[0] + cloud[1] + cloud[2]) / 3.;
        frame[3] = [center.x, center.y, center.z, 1.];
    }
    let inv_a = mat_inverse(&a).ok_or_else(|| bad(id, "invalid moving coordinate frame"))?;
    let inv_b = mat_inverse(&b).ok_or_else(|| bad(id, "invalid reference coordinate frame"))?;
    let (mut dot, mut cross) = (0., 0.);
    for (a, b) in points.into_iter().zip(targets) {
        let p = apply_point(&inv_a, a);
        let q = apply_point(&inv_b, b);
        dot += p.x * q.x + p.y * q.y;
        cross += p.x * q.y - p.y * q.x;
    }
    let spin = rigid(Vec3::ZERO, Vec3::ZERO, Vec3::Z, cross.atan2(dot));
    Ok(mat_mul(&b, &mat_mul(&spin, &inv_a)))
}
#[test]
fn exact_and_noisy_landmarks_and_datum() {
    let id = "test";
    let points = [Vec3::ZERO, Vec3::new(4., 0., 0.), Vec3::new(0., 5., 0.)];
    let moved = rigid(Vec3::new(61., -45., 18.), Vec3::ZERO, Vec3::new(1., 2., 3.), 2.7);
    let targets = points.map(|v| apply_point(&moved, v));
    let fit_matrix = fit(points, targets, id).unwrap();
    assert!(error(points, targets, &fit_matrix) < 1e-11 && is_placement(&fit_matrix));
    let mut noisy = targets;
    noisy[2].x += 0.3;
    let trial = fit(points, noisy, id).unwrap();
    assert!(error(points, noisy, &trial) < error(points, noisy, &moved));
    assert!(basis([Vec3::ZERO, Vec3::X, Vec3::X * 2.], id).is_err());
    let datum = mat_inverse(&basis(targets, id).unwrap()).unwrap();
    assert!(error(targets, points, &datum) < 1e-11);
}

pub(in crate::cmd::mesh_edit) fn landmarks(s: &mut Session, p: &Value) -> Result<Value> {
    let id = "mesh.align_landmarks";
    let args: Landmarks = parse(p, id)?;
    let moving = source(s, id, Some(&args.moving))?;
    let fixed = source(s, id, Some(&args.fixed))?;
    if (moving.feature, moving.index) == (fixed.feature, fixed.index) {
        return Err(bad(id, "choose two distinct imported meshes"));
    }
    let points = triple(args.moving_points, id)?;
    let targets = triple(args.fixed_points, id)?;
    let transform = fit(points, targets, id)?;
    let before = error(points, targets, &IDENTITY);
    let after = error(points, targets, &transform);
    apply_world(s, id, &args.moving, &transform)?;
    Ok(json!(ResultData {
        moving: args.moving,
        rms_before_mm: before,
        rms_after_mm: after,
        align_report: format!("Aligned three landmark pairs. RMS {before:.4} → {after:.4} mm.")
    }))
}
pub(in crate::cmd::mesh_edit) fn datum(s: &mut Session, p: &Value) -> Result<Value> {
    let id = "mesh.align_datum";
    let args: Datum = parse(p, id)?;
    let points = triple([args.origin, args.x_point, args.xy_point], id)?;
    let frame = basis(points, id)?;
    let transform = mat_inverse(&frame).ok_or_else(|| bad(id, "invalid datum frame"))?;
    let x = points[1] - points[0];
    let other = points[2] - points[0];
    let along = x / x.len();
    let targets = [Vec3::ZERO, Vec3::new(x.len(), 0., 0.), Vec3::new(other.dot(along), other.cross(along).len(), 0.)];
    let before = error(points, targets, &IDENTITY);
    let after = error(points, targets, &transform);
    apply_world(s, id, &args.moving, &transform)?;
    Ok(json!(ResultData {
        moving: args.moving,
        rms_before_mm: before,
        rms_after_mm: after,
        align_report: format!("Placed the scan on the global origin, positive X axis and XY plane. Datum RMS {after:.4} mm.")
    }))
}
