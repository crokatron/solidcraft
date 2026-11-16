//! Fine point-to-point ICP on deterministic samples of the two imported scans.
pub(super) mod coarse;
use super::*;
use solidcraft_doc::{IDENTITY, Mat, apply_vector, is_placement, mat_mul, rigid};
use solidcraft_geom::Vec3Geom;
use std::sync::Arc;

const ID: &str = "mesh.align_scans";
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct Params {
    moving: String,
    fixed: String,
    /// Upper limit on rigid refinement steps, 1–100 (default 30).
    iterations: Option<u32>,
}
#[derive(serde::Serialize, JsonSchema)]
pub(super) struct Report {
    moving: String,
    fixed: String,
    rms_before_mm: f64,
    rms_after_mm: f64,
    correspondences: usize,
    iterations: u32,
    align_report: String,
}

fn coord(p: Vec3, k: usize) -> f64 {
    [p.x, p.y, p.z][k]
}

// A balanced kd-tree stored in median order directly in the point array.
fn build(points: &mut [Vec3], depth: usize) {
    if points.len() < 2 {
        return;
    }
    let (left, _, right) = points.select_nth_unstable_by(points.len() / 2, |a, b| coord(*a, depth % 3).total_cmp(&coord(*b, depth % 3)));
    build(left, depth + 1);
    build(right, depth + 1);
}
fn nearest(points: &[Vec3], p: Vec3, depth: usize, best: &mut (f64, Vec3)) {
    if points.is_empty() {
        return;
    }
    let (left, rest) = points.split_at(points.len() / 2);
    let Some((node, right)) = rest.split_first() else { return };
    let distance = (*node - p).len2();
    if distance < best.0 {
        *best = (distance, *node);
    }
    let off = coord(p, depth % 3) - coord(*node, depth % 3);
    let (near, far) = if off <= 0. { (left, right) } else { (right, left) };
    nearest(near, p, depth + 1, best);
    if off * off < best.0 {
        nearest(far, p, depth + 1, best);
    }
}

fn samples(source: &Source<'_>, limit: usize) -> Result<Vec<Vec3>> {
    let corners = &source.mesh.triangles;
    let vertices = source.mesh.positions.as_chunks::<3>().0;
    let count = limit.min(corners.len());
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let id = corners[i * (corners.len() - 1) / count.saturating_sub(1).max(1)];
        let &[x, y, z] = vertices.get(id as usize).ok_or_else(|| bad(ID, "invalid mesh vertex"))?;
        let p = apply_point(&source.frame, Vec3::new(x, y, z));
        if !p.is_finite() {
            return Err(bad(ID, "nonfinite scan point"));
        }
        out.push(p);
    }
    out.sort_unstable_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)).then(a.z.total_cmp(&b.z)));
    out.dedup();
    if out.len() < 3 {
        return Err(bad(ID, "both scans need at least three distinct points"));
    }
    Ok(out)
}

type Match = (f64, Vec3, Vec3);
fn matches(points: &[Vec3], target: &[Vec3], m: &Mat) -> (Vec<Match>, f64) {
    let mut pairs: Vec<_> = points
        .iter()
        .map(|&p| {
            let p = apply_point(m, p);
            let mut hit = (f64::INFINITY, Vec3::ZERO);
            nearest(target, p, 0, &mut hit);
            (hit.0, p, hit.1)
        })
        .collect();
    // Trim the largest residuals on scans with enough samples for partial overlap.
    if pairs.len() >= 30 {
        let keep = pairs.len() * 9 / 10;
        pairs.select_nth_unstable_by(keep, |a, b| a.0.total_cmp(&b.0));
        pairs.truncate(keep);
    }
    let rms = (pairs.iter().map(|r| r.0).sum::<f64>() / pairs.len() as f64).sqrt();
    (pairs, rms)
}

// Solve the three rotational normal equations about the centroids. Re-evaluate exact
// Rodrigues rotations each step; the next match pass catches non-improving steps.
fn step(pairs: &[Match]) -> Result<Mat> {
    let (sum_p, sum_q) = pairs.iter().fold((Vec3::ZERO, Vec3::ZERO), |(p, q), &(_, a, b)| (p + a, q + b));
    let p0 = sum_p / pairs.len() as f64;
    let q0 = sum_q / pairs.len() as f64;
    let mut system = [[0.; 4]; 3];
    for &(_, p, q) in pairs {
        let a = p - p0;
        let b = q - q0;
        let rhs = a.cross(b);
        for (i, row) in system.iter_mut().enumerate() {
            for (j, v) in row[..3].iter_mut().enumerate() {
                *v += (if i == j { a.len2() } else { 0. }) - coord(a, i) * coord(a, j);
            }
            row[3] += coord(rhs, i);
        }
    }
    let scale = system[0][0] + system[1][1] + system[2][2];
    for i in 0..3 {
        let pivot = (i..3).max_by(|&a, &b| system[a][i].abs().total_cmp(&system[b][i].abs())).unwrap_or(i);
        system.swap(i, pivot);
        let div = system[i][i];
        if !div.is_finite() || div.abs() <= scale * 1e-12 {
            return Err(bad(ID, "scan correspondences are collinear or degenerate"));
        }
        for v in &mut system[i][i..] {
            *v /= div;
        }
        let unit = system[i];
        for (k, row) in system.iter_mut().enumerate() {
            if k != i {
                let coeff = row[i];
                for j in i..4 {
                    row[j] -= coeff * unit[j];
                }
            }
        }
    }
    let spin = Vec3::new(system[0][3], system[1][3], system[2][3]);
    Ok(rigid(q0 - p0, p0, spin, spin.len().min(0.35)))
}

#[test]
fn fine_alignment_recovers_a_rigid_scan_transform_and_preserves_reference() {
    let vertices = [Vec3::new(0., 0., 0.), Vec3::new(4., 0., 0.), Vec3::new(0., 5., 0.), Vec3::new(1., 2., 7.)];
    let shift = rigid(Vec3::new(0.14, -0.2, 0.07), Vec3::ZERO, Vec3::new(1., 2., 3.), 0.065);
    let make = |name: &str, transform: &Mat| {
        let p = vertices
            .iter()
            .flat_map(|&p| {
                let v = apply_point(transform, p);
                [v.x, v.y, v.z]
            })
            .collect::<Vec<_>>();
        let n = apply_vector(transform, Vec3::Z);
        MeshData {
            appearance: None,
            name: name.into(),
            positions: p.into(),
            triangles: vec![0, 1, 2, 0, 3, 1, 0, 2, 3, 1, 3, 2].into(),
            color: None,
            corner_normals: (0..12).flat_map(|_| [n.x, n.y, n.z]).collect::<Vec<_>>().into(),
        }
    };
    let mut doc = solidcraft_doc::Document::default();
    let fixed = make("Fixed", &IDENTITY);
    let moving = make("Moving", &shift);
    let feature = doc.add_feature(FeatureKind::MeshImport { file: "proof.stl".into(), meshes: vec![moving, fixed.clone()] }, None).unwrap();
    doc.feature_mut(feature).unwrap().body_names = vec!["Moving".into(), "Fixed".into()];
    let mut s = Session::new(doc);
    s.param_check = crate::schema::ParamCheck::Strict;
    let before = s.doc.to_json();
    let result = s.execute("mesh.align_scans", &json!({"moving":"Moving","fixed":"Fixed"})).unwrap();
    assert!(result["rms_before_mm"].as_f64().unwrap() > 0.1 && result["rms_after_mm"].as_f64().unwrap() < 1e-8, "{result}");
    let FeatureKind::MeshImport { meshes, .. } = &s.doc.feature(feature).unwrap().kind else { panic!() };
    assert_eq!(meshes[1].positions, fixed.positions);
    assert_eq!(meshes[1].triangles, fixed.triangles);
    assert_eq!(meshes[1].corner_normals, fixed.corner_normals);
    for (a, b) in meshes[0].positions.iter().zip(fixed.positions.iter()).chain(meshes[0].corner_normals.iter().zip(fixed.corner_normals.iter())) {
        assert!((a - b).abs() < 1e-8, "{a} {b}");
    }
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc.to_json(), before);
    for args in [json!({"moving":"Moving","fixed":"Moving"}), json!({"moving":"Moving","fixed":"Fixed","iterations":0})] {
        assert!(s.execute("mesh.align_scans", &args).is_err());
        assert_eq!(s.doc.to_json(), before);
    }
}

mod landmarks;
pub(super) mod plane;
pub(super) use landmarks::{Datum, Landmarks, ResultData, datum, landmarks};

fn apply_world(s: &mut Session, id: &str, name: &str, transform: &Mat) -> Result<()> {
    if !is_placement(transform) {
        return Err(bad(id, "invalid rigid transform"));
    }
    let moving = source(s, id, Some(name))?;
    let frames = s.body_frames();
    if let Some((_, own, _)) = frames.iter().find(|(world, ..)| world == name)
        && frames.iter().filter(|(_, source, _)| source == own).count() > 1
    {
        return Err(bad(id, "the moving scan has shared occurrences; make an independent copy before aligning"));
    }
    let inverse = mat_inverse(&moving.frame).ok_or_else(|| bad(id, "moving scan has a singular placement"))?;
    let local = mat_mul(&mat_mul(&inverse, transform), &moving.frame);
    if !solidcraft_doc::is_identity(&local) {
        plane::move_linked(s, moving.feature, moving.index, transform)?;
        edit(s, id, Some(name), |mesh, _| {
            for xyz in Arc::make_mut(&mut mesh.positions).as_chunks_mut::<3>().0 {
                let p = apply_point(&local, Vec3::new(xyz[0], xyz[1], xyz[2]));
                if !p.is_finite() {
                    return Err(bad(id, "alignment produced a nonfinite vertex"));
                }
                *xyz = [p.x, p.y, p.z];
            }
            for xyz in Arc::make_mut(&mut mesh.corner_normals).as_chunks_mut::<3>().0 {
                let n = apply_vector(&local, Vec3::new(xyz[0], xyz[1], xyz[2]));
                *xyz = [n.x, n.y, n.z];
            }
            Ok(())
        })?;
    }
    Ok(())
}

pub(super) fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let args: Params = parse(p, ID)?;
    let limit = args.iterations.unwrap_or(30);
    if !(1..=100).contains(&limit) {
        return Err(bad(ID, "iterations must be 1 to 100"));
    }
    let moving = source(s, ID, Some(&args.moving))?;
    let fixed = source(s, ID, Some(&args.fixed))?;
    if (moving.feature, moving.index) == (fixed.feature, fixed.index) {
        return Err(bad(ID, "choose two distinct imported meshes"));
    }

    let points = samples(&moving, 8192)?;
    let mut target = samples(&fixed, 32768)?;
    let start = *target.first().ok_or_else(|| bad(ID, "fixed scan has no sample points"))?;
    let lo = target.iter().copied().fold(start, Vec3::min);
    let hi = target.iter().copied().fold(start, Vec3::max);
    let scale = (hi - lo).len();
    let axis = target.iter().map(|&p| p - start).max_by(|a, b| a.len2().total_cmp(&b.len2())).unwrap_or(Vec3::ZERO);
    if !scale.is_finite() || !target.iter().any(|&p| axis.cross(p - start).len() > scale * scale * 1e-12) {
        return Err(bad(ID, "fixed scan points must span at least two directions"));
    }
    build(&mut target, 0);
    let mut transform = IDENTITY;
    let (mut pairs, initial) = matches(&points, &target, &transform);
    if initial > scale * 0.25 {
        return Err(bad(ID, "bring the overlapping scans roughly into alignment first"));
    }
    let mut rms = initial;
    let mut steps = 0;
    for _ in 0..limit {
        let trial = mat_mul(&step(&pairs)?, &transform);
        let (next, error) = matches(&points, &target, &trial);
        if !error.is_finite() || error > rms {
            break;
        }
        let gain = rms - error;
        (transform, pairs, rms) = (trial, next, error);
        steps += 1;
        if gain <= scale * 1e-9 {
            break;
        }
    }
    if !is_placement(&transform) {
        return Err(bad(ID, "invalid fitted rigid transform"));
    }
    if rms >= initial && initial > scale * 1e-9 {
        return Err(bad(ID, "alignment did not improve; bring the scans closer first"));
    }
    apply_world(s, ID, &args.moving, &transform)?;
    Ok(json!(Report {
        moving: args.moving,
        fixed: args.fixed,
        rms_before_mm: initial,
        rms_after_mm: rms,
        iterations: steps,
        correspondences: pairs.len(),
        align_report: format!("Sample RMS {initial:.4} → {rms:.4} mm over {} matches in {steps} iterations.", pairs.len())
    }))
}
