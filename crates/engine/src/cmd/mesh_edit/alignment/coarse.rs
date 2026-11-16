use super::*;

const COARSE: &str = "mesh.align_coarse";

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(in crate::cmd::mesh_edit) struct Args {
    moving: String,
    fixed: String,
}

fn frame(points: &[Vec3]) -> Result<(Vec3, [Vec3; 3])> {
    let center = points.iter().copied().fold(Vec3::ZERO, |a, b| a + b) / points.len() as f64;
    let mut covariance = [[0.; 3]; 3];
    for &point in points {
        let v = point - center;
        for (i, row) in covariance.iter_mut().enumerate() {
            for (j, entry) in row.iter_mut().enumerate() {
                *entry += coord(v, i) * coord(v, j);
            }
        }
    }
    let (values, basis) = principal_axes(covariance);
    if values[1] <= values[2] * 1e-12 || values[2] <= 0. {
        return Err(bad(COARSE, "each scan must span at least two directions"));
    }
    let [a, b, _] = basis.map(|v| Vec3::new(v[0], v[1], v[2]));
    Ok((center, [a, b, a.cross(b)]))
}

pub(in crate::cmd::mesh_edit) fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let args: Args = parse(p, COARSE)?;
    let moving = source(s, COARSE, Some(&args.moving))?;
    let fixed = source(s, COARSE, Some(&args.fixed))?;
    if (moving.feature, moving.index) == (fixed.feature, fixed.index) {
        return Err(bad(COARSE, "choose two distinct imported meshes"));
    }
    let points = samples(&moving, 8192)?;
    let mut target = samples(&fixed, 8192)?;
    let (center, basis) = frame(&points)?;
    let (goal, axes) = frame(&target)?;
    build(&mut target, 0);
    let initial = matches(&points, &target, &IDENTITY).1;
    let mut best = IDENTITY;
    let mut rms = initial;
    // A principal axis has no sign and nearly equal axes may trade places.
    // Try all 24 right-handed frames against the actual sampled surfaces.
    for (i, j) in [(0, 1), (1, 0), (0, 2), (2, 0), (1, 2), (2, 1)] {
        for u in [-1., 1.] {
            for v in [-1., 1.] {
                let a = axes[i] * u;
                let b = axes[j] * v;
                let dest = [a, b, a.cross(b)];
                let mut transform = IDENTITY;
                for (row, r) in transform[..3].iter_mut().enumerate() {
                    for (col, entry) in r[..3].iter_mut().enumerate() {
                        *entry = (0..3).map(|k| coord(dest[k], col) * coord(basis[k], row)).sum();
                    }
                }
                let shift = goal - apply_vector(&transform, center);
                transform[3][..3].copy_from_slice(&[shift.x, shift.y, shift.z]);
                let score = matches(&points, &target, &transform).1;
                if score < rms {
                    best = transform;
                    rms = score;
                }
            }
        }
    }
    let (mut pairs, _) = matches(&points, &target, &best);
    let mut steps = 0;
    for _ in 0..12 {
        let Ok(delta) = step(&pairs) else { break };
        let trial = mat_mul(&delta, &best);
        let (next, error) = matches(&points, &target, &trial);
        if !error.is_finite() || error >= rms {
            break;
        }
        let gain = rms - error;
        (best, pairs, rms) = (trial, next, error);
        steps += 1;
        if gain < 1e-8 {
            break;
        }
    }
    apply_world(s, COARSE, &args.moving, &best)?;
    Ok(json!(Report {
        moving: args.moving,
        fixed: args.fixed,
        rms_before_mm: initial,
        rms_after_mm: rms,
        correspondences: pairs.len(),
        iterations: steps,
        align_report: format!(
            "Auto alignment sampled RMS {initial:.4} → {rms:.4} mm across {} matches. Check orientation on symmetric scans.",
            pairs.len()
        )
    }))
}
