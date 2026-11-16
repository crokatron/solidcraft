//! Bounded analytic surfaces fitted to a connected scan region, retaining its trim loops.
//!
//! Original construction. Orthogonal least squares and normal-plane initialization follow
//! the geometric formulations described by Eberly, Least Squares Fitting (2024), sections
//! 4, 7 and 8. No external implementation is copied. Scan residuals are measured samples,
//! never a certificate for the continuous mesh or for an unmeasured scan.
//! Inputs are bounded to two million triangles; boundary walks and simplification are iterative.
use crate::geom::matrix::Matrix;
use crate::{Brep, Kernel, Result, SolidError, Vec3};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[path = "scan_surface_fit/trim.rs"]
mod trim;
pub use trim::{ordered_uv_loops, trimmed_sheet};
#[path = "scan_surface_fit/primitive.rs"]
mod primitive;

const OP: &str = "query::scan_surface_fit";
pub(super) fn invalid(why: &'static str) -> SolidError {
    SolidError::invalid_input(OP, why)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
/// Target family for a bounded scan patch.
pub enum Kind {
    /// Least-squares plane.
    Plane,
    /// Circular cylinder with a fitted axis and radius.
    Cylinder,
    /// Circular cone with a fitted apex, axis and half-angle.
    Cone,
    /// One bicubic height-graph patch.
    Freeform,
}
#[derive(Clone, Debug)]
/// User controls in the input coordinate frame and model units (millimetres).
pub struct Options {
    /// Requested surface family.
    pub kind: Kind,
    /// Positive target error; compared to measured maximum, never used to hide poor fits.
    pub tolerance: f64,
    /// Maximum chord simplification deviation in model units; zero retains every edge.
    pub boundary_tolerance: f64,
    /// Optional direction for the fitted surface U axis, in the input frame.
    pub flow: Option<[f64; 3]>,
    /// Dimensionless freeform regularization strength in [0, 1].
    pub fairing: f64,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            kind: Kind::Plane,
            tolerance: 0.5,
            boundary_tolerance: 0.5,
            flow: None,
            fairing: 0.0001,
        }
    }
}
#[derive(Clone, Debug, Serialize)]
/// Measured support-surface residuals; these are not continuous error certificates.
pub struct Report {
    /// Lowercase fitted family name.
    pub kind: String,
    /// Root mean square over the explicitly measured samples.
    pub rms_mm: f64,
    /// Largest absolute residual among the measured samples.
    pub max_mm: f64,
    /// Whether the measured maximum is within the requested tolerance.
    pub within_tolerance: bool,
    /// Retained outer and hole loop count.
    pub boundary_loops: usize,
    /// Total retained boundary vertex count.
    pub boundary_vertices: usize,
    /// Freeform U control count, or zero for analytic families.
    pub control_points_u: usize,
    /// Freeform V control count, or zero for analytic families.
    pub control_points_v: usize,
    /// Named fitted dimensions and frame components.
    pub parameters: BTreeMap<String, f64>,
    /// Human-readable measurement direction and limitations.
    pub metric: String,
    /// Number of measured vertices and facet centres.
    pub sampled_points: usize,
    /// Number of source region triangles.
    pub region_triangles: usize,
}
#[derive(Clone, Debug)]
/// Connected, orientable, exactly welded region with ordered boundary loops.
pub struct PreparedRegion {
    /// Compacted exact-coordinate-welded points.
    pub positions: Vec<Vec3>,
    /// Consistently oriented triangles indexing positions.
    pub triangles: Vec<[usize; 3]>,
    /// Closed boundary walks without a repeated closing vertex.
    pub loops: Vec<Vec<usize>>,
    /// At most 15000 area-weighted vertex samples for fitting.
    pub samples: Vec<(Vec3, f64)>,
    /// Area-weighted facet centroid.
    pub center: Vec3,
    /// Least-variance position axis oriented toward the region normals.
    pub normal: Vec3,
    /// Largest-variance in-plane frame direction.
    pub x: Vec3,
    /// Right-handed frame direction normal cross x.
    pub y: Vec3,
    /// Sampled unit facet normals, centroids and twice-area weights.
    pub face_normals: Vec<(Vec3, Vec3, f64)>,
}
/// Compact, exact-coordinate-welded topology; duplicated texture seams do not create holes.
pub fn prepare_region(positions: &[[f64; 3]], triangles: &[[u32; 3]]) -> Result<PreparedRegion> {
    if triangles.is_empty() || triangles.len() > 2_000_000 || positions.len() > 6_000_000 {
        return Err(invalid(
            "scan fitting supports 1 to 2 million selected triangles",
        ));
    }
    let mut points = Vec::new();
    let mut welded = BTreeMap::new();
    let mut remap = BTreeMap::new();
    let mut faces = Vec::new();
    let mut unique = BTreeSet::new();
    for tri in triangles {
        let mut ids = [0usize; 3];
        for (slot, &index) in ids.iter_mut().zip(tri) {
            let mapped = if let Some(&v) = remap.get(&index) {
                v
            } else {
                let p = positions
                    .get(index as usize)
                    .ok_or_else(|| invalid("scan vertex index is out of range"))?;
                if !p.iter().all(|v| v.is_finite() && v.abs() < 1e9) {
                    return Err(invalid(
                        "scan coordinates must be finite and within modelling limits",
                    ));
                }
                let key = p.map(|x| if x == 0. { 0 } else { x.to_bits() });
                let v = if let Some(&v) = welded.get(&key) {
                    v
                } else {
                    let v = points.len();
                    points.push(Vec3::from_array(*p));
                    welded.insert(key, v);
                    v
                };
                remap.insert(index, v);
                v
            };
            *slot = mapped;
        }
        let mut sorted = ids;
        sorted.sort_unstable();
        if sorted[0] == sorted[1] || sorted[1] == sorted[2] || !unique.insert(sorted) {
            return Err(invalid(
                "selected region has collapsed or duplicate triangles; clean the scan first",
            ));
        }
        faces.push(ids);
    }
    let point = |i: usize| {
        points
            .get(i)
            .copied()
            .ok_or_else(|| invalid("missing scan vertex"))
    };
    let mut edges: BTreeMap<(usize, usize), Vec<(usize, usize, usize)>> = BTreeMap::new();
    for (fi, t) in faces.iter().enumerate() {
        let a = point(t[0])?;
        let b = point(t[1])?;
        let c = point(t[2])?;
        if (b - a).cross(c - a).length() < 1e-14 {
            return Err(invalid("selected region has zero-area triangles"));
        }
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            let list = edges.entry((a.min(b), a.max(b))).or_default();
            list.push((fi, a, b));
            if list.len() > 2 {
                return Err(invalid("selected region is non-manifold"));
            }
        }
    }
    let mut adjacency = vec![Vec::new(); faces.len()];
    for uses in edges.values() {
        if let [a, b] = uses.as_slice() {
            adjacency
                .get_mut(a.0)
                .ok_or_else(|| invalid("missing adjacent face"))?
                .push((b.0, a.1 == b.1));
            adjacency
                .get_mut(b.0)
                .ok_or_else(|| invalid("missing adjacent face"))?
                .push((a.0, a.1 == b.1));
        }
    }
    let mut orientations = vec![0i8; faces.len()];
    if let Some(first) = orientations.first_mut() {
        *first = 1;
    }
    let mut queue = VecDeque::from([0usize]);
    let mut visited = 0;
    while let Some(fi) = queue.pop_front() {
        visited += 1;
        let sign = *orientations
            .get(fi)
            .ok_or_else(|| invalid("missing orientation"))?;
        for &(other, same) in adjacency
            .get(fi)
            .ok_or_else(|| invalid("missing adjacency"))?
        {
            let needed = if same { -sign } else { sign };
            let orientation = orientations
                .get_mut(other)
                .ok_or_else(|| invalid("missing orientation"))?;
            if *orientation == 0 {
                *orientation = needed;
                queue.push_back(other);
            } else if *orientation != needed {
                return Err(invalid("selected region is not orientable"));
            }
        }
    }
    if visited != faces.len() {
        return Err(invalid(
            "select one connected scan region; disconnected areas need separate surfaces",
        ));
    }
    let mut next = BTreeMap::new();
    let mut incoming = BTreeSet::new();
    for uses in edges.values() {
        if let [entry] = uses.as_slice() {
            let (_, mut a, mut b) = *entry;
            if orientations.get(entry.0).copied().unwrap_or(0) < 0 {
                std::mem::swap(&mut a, &mut b);
            }
            if next.insert(a, b).is_some() || !incoming.insert(b) {
                return Err(invalid("scan boundary branches or touches itself"));
            }
        }
    }
    if next.is_empty() {
        return Err(invalid(
            "select an open patch with a boundary, rather than a closed scan",
        ));
    }
    if next.len() > 100_000 {
        return Err(invalid(
            "selected scan boundary exceeds 100000 edges; simplify the selection first",
        ));
    }
    let mut loops = Vec::new();
    while let Some((&start, _)) = next.first_key_value() {
        let mut ring = Vec::new();
        let mut current = start;
        loop {
            ring.push(current);
            current = next
                .remove(&current)
                .ok_or_else(|| invalid("scan boundary is not closed"))?;
            if current == start {
                break;
            }
            if ring.len() > 100_000 {
                return Err(invalid("scan boundary walk exceeded its bound"));
            }
        }
        if ring.len() < 3 {
            return Err(invalid("scan boundary has fewer than three vertices"));
        }
        loops.push(ring);
        if loops.len() > 128 {
            return Err(invalid(
                "more than128 boundary loops; clean or split this region first",
            ));
        }
    }
    for (t, &sign) in faces.iter_mut().zip(&orientations) {
        if sign < 0 {
            t.swap(1, 2);
        }
    }
    let step = faces.len().div_ceil(5000).max(1);
    let mut samples = Vec::new();
    let mut normals = Vec::new();
    let mut center = Vec3::ZERO;
    let mut weight = 0.;
    let mut average_normal = Vec3::ZERO;
    for (i, t) in faces.iter().enumerate() {
        let a = point(t[0])?;
        let b = point(t[1])?;
        let c = point(t[2])?;
        let cross = (b - a).cross(c - a);
        let area = cross.length();
        let mid = (a + b + c) / 3.;
        center += mid * area;
        weight += area;
        average_normal += cross;
        if i % step == 0 {
            for p in [a, b, c] {
                samples.push((p, area / 3.));
            }
            normals.push((cross / area, mid, area));
        }
    }
    if !weight.is_finite() || weight <= 0. {
        return Err(invalid("selected scan region has no finite area"));
    }
    center = center / weight;
    let mut covariance = Matrix::new(3, 3);
    for &(p, w) in &samples {
        let d = p - center;
        for row in 0..3 {
            for col in row..3 {
                let x = covariance.at(row, col)? + w * d.component(row) * d.component(col);
                covariance.set_at(row, col, x)?;
                covariance.set_at(col, row, x)?;
            }
        }
    }
    let (values, axes) = covariance.compute_3x3_eigen_vectors()?;
    if values[1] <= values[2].abs() * 1e-12 {
        return Err(invalid(
            "selected scan region is effectively a line, not a surface",
        ));
    }
    let mut normal = axes[0].unitize()?;
    if normal.dot(average_normal) < 0. {
        normal = -normal;
    }
    let x = axes[2].unitize()?;
    let y = normal.cross(x).unitize()?;
    Ok(PreparedRegion {
        positions: points,
        triangles: faces,
        loops,
        samples,
        center,
        normal,
        x,
        y,
        face_normals: normals,
    })
}
/// Simplify the observed boundary in model space; loops are never dropped or merged.
pub fn simplify_region_boundaries(region: &mut PreparedRegion, tolerance: f64) -> Result<()> {
    if !tolerance.is_finite() || tolerance < 0. {
        return Err(invalid("boundary tolerance must be finite and nonnegative"));
    }
    if tolerance > 0. {
        for ring in &mut region.loops {
            let n = ring.len();
            let mid = n / 2;
            let mut closed = ring.clone();
            let first = *ring.first().ok_or_else(|| invalid("empty boundary"))?;
            closed.push(first);
            let mut keep = BTreeSet::from([0, mid, n]);
            let mut stack = vec![(0, mid), (mid, n)];
            while let Some((a, b)) = stack.pop() {
                if b <= a + 1 {
                    continue;
                }
                let pa = *region
                    .positions
                    .get(*closed.get(a).ok_or_else(|| invalid("missing boundary"))?)
                    .ok_or_else(|| invalid("missing point"))?;
                let pb = *region
                    .positions
                    .get(*closed.get(b).ok_or_else(|| invalid("missing boundary"))?)
                    .ok_or_else(|| invalid("missing point"))?;
                let delta = pb - pa;
                let length2 = delta.length_squared();
                let mut best = tolerance;
                let mut split = None;
                for i in a + 1..b {
                    let p = *region
                        .positions
                        .get(*closed.get(i).ok_or_else(|| invalid("missing boundary"))?)
                        .ok_or_else(|| invalid("missing point"))?;
                    let t = if length2 > 1e-30 {
                        ((p - pa).dot(delta) / length2).clamp(0., 1.)
                    } else {
                        0.
                    };
                    let distance = (p - pa - delta * t).length();
                    if distance > best {
                        best = distance;
                        split = Some(i);
                    }
                }
                if let Some(i) = split {
                    keep.insert(i);
                    stack.push((a, i));
                    stack.push((i, b));
                }
            }
            let simplified: Vec<_> = keep
                .into_iter()
                .filter(|&i| i < n)
                .filter_map(|i| closed.get(i).copied())
                .collect();
            // Preserve small loops exactly rather than removing a hole when simplification
            // has fewer than three survivors. The total edge budget still applies below.
            if simplified.len() >= 3 {
                *ring = simplified;
            }
        }
    }
    if region.loops.iter().map(Vec::len).sum::<usize>() > 4096 {
        return Err(invalid(
            "fitted surface boundary exceeds4096 edges; increase boundary tolerance or select a smaller patch",
        ));
    }
    Ok(())
}
/// Fit one selected region to a native trimmed sheet, preserving holes and reporting poor fits.
pub fn fit(
    k: &mut Kernel,
    positions: &[[f64; 3]],
    triangles: &[[u32; 3]],
    options: &Options,
) -> Result<(Brep, Report)> {
    if !options.tolerance.is_finite()
        || options.tolerance <= 0.
        || options
            .flow
            .is_some_and(|v| !v.iter().all(|x| x.is_finite()))
        || !options.fairing.is_finite()
        || !(0. ..=1.).contains(&options.fairing)
    {
        return Err(invalid(
            "fit tolerance must be positive and fairing must be between zero and one",
        ));
    }
    let mut region = prepare_region(positions, triangles)?;
    simplify_region_boundaries(&mut region, options.boundary_tolerance)?;
    if options.kind == Kind::Freeform {
        let (body, p) = super::scan_surface_patch::fit_freeform(
            k,
            &region,
            options.flow,
            options.tolerance,
            options.fairing,
        )?;
        return Ok((
            body,
            Report {
                kind: "freeform".into(),
                rms_mm: p.rms,
                max_mm: p.max,
                within_tolerance: p.max <= options.tolerance,
                boundary_loops: region.loops.len(),
                boundary_vertices: p.boundary_edges,
                control_points_u: 4,
                control_points_v: 4,
                parameters: BTreeMap::new(),
                metric: "frame_normal_sampled".into(),
                sampled_points: p.measured_samples,
                region_triangles: region.triangles.len(),
            },
        ));
    }
    primitive::fit(k, &region, options)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn grid(kind: Kind, hole: bool) -> (Vec<[f64; 3]>, Vec<[u32; 3]>) {
        let mut p = Vec::new();
        let mut t = Vec::new();
        for i in 0..17 {
            for j in 0..9 {
                let angle = -0.8 + i as f64 * 0.1;
                let h = 20. + j as f64;
                let q = match kind {
                    Kind::Plane => [i as f64, j as f64, 0.],
                    Kind::Cylinder => [10. * angle.cos(), 10. * angle.sin(), h],
                    _ => [0.4 * h * angle.cos(), 0.4 * h * angle.sin(), h],
                };
                p.push(q);
            }
        }
        for i in 0..16 {
            for j in 0..8 {
                if hole && (6..9).contains(&i) && (3..5).contains(&j) {
                    continue;
                }
                let a = i * 9 + j;
                t.extend([[a, a + 9, a + 10], [a, a + 10, a + 1]]);
            }
        }
        (p, t)
    }
    #[test]
    fn plane_hole() {
        let (p, t) = grid(Kind::Plane, true);
        let (body, r) = fit(
            &mut Kernel::default(),
            &p,
            &t,
            &Options {
                boundary_tolerance: 0.001,
                ..Options::default()
            },
        )
        .unwrap();
        assert_eq!(body.faces().len(), 1);
        assert_eq!(r.boundary_loops, 2);
        assert_eq!(body.edges().len(), 8);
        assert!(r.max_mm < 1e-8);
    }
    #[test]
    fn cylinder_sheet() {
        let (p, t) = grid(Kind::Cylinder, false);
        let (body, r) = fit(
            &mut Kernel::default(),
            &p,
            &t,
            &Options {
                kind: Kind::Cylinder,
                boundary_tolerance: 0.001,
                ..Options::default()
            },
        )
        .unwrap();
        eprintln!("cyl {r:?}");
        assert_eq!(body.faces().len(), 1);
        assert!((r.parameters["radius_start_mm"] - 10.).abs() < 1e-5);
        assert!(r.max_mm < 0.02);
    }
    #[test]
    fn cone_sheet() {
        let (p, t) = grid(Kind::Cone, false);
        let (body, r) = fit(
            &mut Kernel::default(),
            &p,
            &t,
            &Options {
                kind: Kind::Cone,
                boundary_tolerance: 0.001,
                ..Options::default()
            },
        )
        .unwrap();
        eprintln!("cone {r:?}");
        assert_eq!(body.faces().len(), 1);
        assert!((r.parameters["half_angle_deg"] - 0.4_f64.atan().to_degrees()).abs() < 0.01);
        assert!(r.max_mm < 0.02);
    }
    #[test]
    fn refuses_disconnected() {
        let (p, mut t) = grid(Kind::Plane, false);
        t.retain(|v| v.iter().all(|i| *i < 50) || v.iter().all(|i| *i > 100));
        assert!(prepare_region(&p, &t).is_err());
    }
    #[test]
    fn noisy_arbitrary_axis_cylinder() {
        let (mut p, t) = grid(Kind::Cylinder, true);
        let axis = Vec3::new(1., 2., 3.).unitize().unwrap();
        let x = Vec3::Y.cross(axis).unitize().unwrap();
        let y = axis.cross(x);
        for (i, p) in p.iter_mut().enumerate() {
            let noise = ((i * 31 % 19) as f64 - 9.) * 0.001;
            let q = x * p[0] + y * p[1] + axis * p[2] + Vec3::new(120., -20., 40.) + x * noise;
            *p = [q.x, q.y, q.z];
        }
        let (body, r) = fit(
            &mut Kernel::default(),
            &p,
            &t,
            &Options {
                kind: Kind::Cylinder,
                boundary_tolerance: 0.001,
                tolerance: 0.1,
                ..Options::default()
            },
        )
        .unwrap();
        assert_eq!(body.faces().len(), 1);
        assert_eq!(r.boundary_loops, 2);
        assert!((r.parameters["radius_start_mm"] - 10.).abs() < 0.03);
        assert!(r.within_tolerance);
        let got = Vec3::new(
            r.parameters["axis_x"],
            r.parameters["axis_y"],
            r.parameters["axis_z"],
        );
        assert!(got.dot(axis).abs() > 0.9999);
    }
    #[test]
    fn poor_fit_reports_not_hides_error() {
        let (mut p, t) = grid(Kind::Plane, false);
        for p in &mut p {
            p[2] = 0.05 * p[0] * p[0];
        }
        let (_, r) = fit(
            &mut Kernel::default(),
            &p,
            &t,
            &Options {
                tolerance: 0.001,
                ..Options::default()
            },
        )
        .unwrap();
        assert!(!r.within_tolerance);
        assert!(r.max_mm > 0.1);
    }
    #[test]
    fn rejects_invalid_triangle() {
        assert!(prepare_region(&[[0., 0., 0.]], &[[0, 1, 2]]).is_err());
    }
    #[test]
    fn primitive_native_topology_and_geometry_checks() {
        use crate::topology::brep::p2::ArenaNotifyState;
        use crate::topology::brep::validation::{CheckLevel, check_brep_with_keys};
        for kind in [Kind::Plane, Kind::Cylinder, Kind::Cone] {
            let (p, t) = grid(kind, true);
            let mut k = Kernel::default();
            let (mut body, _) = fit(
                &mut k,
                &p,
                &t,
                &Options {
                    kind,
                    boundary_tolerance: 0.001,
                    ..Options::default()
                },
            )
            .unwrap();
            let mut notify = ArenaNotifyState::new(&body);
            let env = notify.env(None);
            let check =
                check_brep_with_keys(&mut k, &mut body, CheckLevel::Level1, &|e| (env.key_of)(e))
                    .unwrap();
            assert!(check.is_valid(), "{kind:?}: {:?}", check.lines());
        }
    }
    #[test]
    fn tiny_hole_survives_large_boundary_tolerance() {
        let (p, t) = grid(Kind::Plane, true);
        let mut region = prepare_region(&p, &t).unwrap();
        let original = region.loops.clone();
        simplify_region_boundaries(&mut region, 100.).unwrap();
        assert_eq!(region.loops, original);
    }
    #[test]
    fn invalid_controls_are_refused() {
        let (p, t) = grid(Kind::Plane, false);
        for options in [
            Options {
                fairing: 1.1,
                ..Options::default()
            },
            Options {
                flow: Some([f64::NAN, 0., 0.]),
                ..Options::default()
            },
        ] {
            assert!(fit(&mut Kernel::default(), &p, &t, &options).is_err());
        }
    }
    #[test]
    fn collinear_uv_subdivisions_merge_without_smoothing_corners() {
        let ring = vec![
            [0., 0.],
            [1., 0.],
            [2., 0.],
            [2., 1.],
            [2., 2.],
            [1., 2.],
            [0., 2.],
            [0., 1.],
        ];
        let compact = ordered_uv_loops(vec![ring]).unwrap();
        assert_eq!(compact[0].len(), 4);
        let curved = vec![[0., 0.], [1., -0.001], [2., 0.], [2., 2.], [0., 2.]];
        let compact = ordered_uv_loops(vec![curved]).unwrap();
        assert_eq!(compact[0].len(), 5);
    }
    #[test]
    fn aligned_curved_patch_has_four_outer_and_four_hole_edges() {
        let (mut p, t) = grid(Kind::Plane, true);
        for p in &mut p {
            p[2] = 0.01 * p[0] * p[0] + 0.02 * p[1] * p[1];
        }
        let mut region = prepare_region(&p, &t).unwrap();
        region.center = Vec3::ZERO;
        region.normal = Vec3::Z;
        region.x = Vec3::X;
        region.y = Vec3::Y;
        let (body, report) = crate::query::scan_surface_patch::fit_freeform(
            &mut Kernel::default(),
            &region,
            Some([1., 0., 0.]),
            0.5,
            0.,
        )
        .unwrap();
        assert_eq!(body.edges().len(), 8);
        assert_eq!(report.boundary_edges, 8);
        assert_eq!(report.boundary_loops, 2);
    }
    /// The native trimmed sheet must retain concavity instead of filling its bounding box.
    #[test]
    fn concave_plane_preserves_missing_quadrant() {
        let mut points = Vec::new();
        let mut triangles = Vec::new();
        for i in 0..3 {
            for j in 0..3 {
                points.push([i as f64 * 20., j as f64 * 20., 0.]);
            }
        }
        for i in 0..2 {
            for j in 0..2 {
                if i == 1 && j == 1 {
                    continue;
                }
                let a = i * 3 + j;
                triangles.extend([[a, a + 3, a + 4], [a, a + 4, a + 1]]);
            }
        }
        let mut kernel = Kernel::default();
        let (mut body, report) = fit(&mut kernel, &points, &triangles, &Options::default()).unwrap();
        assert_eq!(report.boundary_loops, 1);
        assert_eq!(body.edges().len(), 6);
        let area = crate::mass::integrator::properties::brep_area(&kernel, &mut body, 1e-8).unwrap();
        assert!(
            (area - 1200.).abs() < 1e-5,
            "area={area}; a filled bounding box would be 1600"
        );
    }
}
