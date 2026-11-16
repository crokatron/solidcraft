//! Bounded, low-complexity freeform reconstruction of one scan region.
//!
//! Original construction: weighted least squares in tensor Bernstein basis with a
//! dimensionless second-difference fairness penalty. The 4x4 bicubic net keeps its
//! two planar coordinates affine; only height is fitted. Its planar Jacobian is
//! therefore positive everywhere: this patch cannot fold over itself. Straight UV
//! boundary segments lift exactly to degree-six polynomial curves by the Bernstein
//! product identity, not by tessellation or a fit to boundary samples.
//!
//! Deviations: one graph chart only, no G1/G2 patch matching or Class-A claim. Input
//! topology and limits belong to `scan_surface_fit::prepare_region`; fit samples
//! are capped there. All vertices and triangle centroids are checked for normal-
//! direction residual, which is sampled error, not a Hausdorff certificate. No recursion.

use super::scan_surface_fit::{PreparedRegion, ordered_uv_loops, trimmed_sheet};
use crate::curve::curve::Curve;
use crate::kernel::Kernel;
use crate::nurbs::b_spline_curve::BSplineCurve;
use crate::nurbs::b_spline_surface::BSplineSurface;
use crate::nurbs::crv_basic::Curve as NurbCurve;
use crate::nurbs::math::{CPoint, MatrixType, RealMatrix};
use crate::nurbs::srf_basic::Surface as NurbSurface;
use crate::surface::surface::Surface;
use crate::topology::Brep;
use crate::{Result, SolidError, Vec3};

/// Diagnostics of the sampled scan-to-graph fit, in model units.
#[derive(Clone, Debug)]
pub struct PatchReport {
    /// Number of area-weighted samples used to fit the surface.
    pub fit_samples: usize,
    /// All region vertices plus triangle centroids, measured after fitting.
    pub measured_samples: usize,
    /// Root-mean-square frame-normal residual at those measurement points.
    pub rms: f64,
    /// Largest absolute frame-normal residual at those measurement points.
    pub max: f64,
    /// The U flow direction, in the input coordinate frame.
    pub flow: [f64; 3],
    /// The fitted graph's height direction.
    pub normal: [f64; 3],
    /// Always four by four for this bounded initial freeform construction.
    pub control_points: [usize; 2],
    /// Number of retained trim loops, including holes.
    pub boundary_loops: usize,
    /// Number of exact degree-six trim curves.
    pub boundary_edges: usize,
    /// Dimensionless regularization weight actually used.
    pub fairing: f64,
}
fn invalid(reason: &'static str) -> SolidError {
    SolidError::invalid_input("query::scan_surface_patch", reason)
}
fn get<T: Copy>(values: &[T], i: usize) -> Result<T> {
    values
        .get(i)
        .copied()
        .ok_or_else(|| invalid("invalid prepared scan index"))
}
fn finite(p: Vec3) -> bool {
    p.x.is_finite() && p.y.is_finite() && p.z.is_finite()
}
fn array(p: Vec3) -> [f64; 3] {
    [p.x, p.y, p.z]
}
fn basis(t: f64) -> [f64; 4] {
    let s = 1. - t;
    [s * s * s, 3. * t * s * s, 3. * t * t * s, t * t * t]
}
fn tensor(u: f64, v: f64) -> Vec<f64> {
    let (a, b) = (basis(u), basis(v));
    a.into_iter()
        .flat_map(|x| b.into_iter().map(move |y| x * y))
        .collect()
}
#[derive(Clone, Copy)]
struct Frame {
    origin: Vec3,
    x: Vec3,
    y: Vec3,
    n: Vec3,
    lo: [f64; 2],
    size: [f64; 2],
    scale: f64,
}
impl Frame {
    fn coordinates(self, p: Vec3) -> [f64; 3] {
        let d = p - self.origin;
        [
            (d.dot(self.x) - self.lo[0]) / self.size[0],
            (d.dot(self.y) - self.lo[1]) / self.size[1],
            d.dot(self.n) / self.scale,
        ]
    }
    fn point(self, u: f64, v: f64, height: f64) -> Vec3 {
        self.origin
            + self.x * (self.lo[0] + self.size[0] * u)
            + self.y * (self.lo[1] + self.size[1] * v)
            + self.n * (height * self.scale)
    }
}
fn frame(region: &PreparedRegion, flow: Option<[f64; 3]>) -> Result<Frame> {
    let n = region.normal.unitize()?;
    let desired = flow.map(Vec3::from).unwrap_or(region.x);
    if !finite(desired) || !finite(region.center) {
        return Err(invalid("invalid freeform flow direction"));
    }
    let x = (desired - n * desired.dot(n)).unitize().map_err(|_| {
        invalid("flow direction must have a component along the selected scan region")
    })?;
    let y = n.cross(x).unitize()?;
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    for &p in &region.positions {
        if !finite(p) {
            return Err(invalid("non-finite scan point"));
        }
        let d = p - region.center;
        let q = [d.dot(x), d.dot(y)];
        lo[0] = lo[0].min(q[0]);
        lo[1] = lo[1].min(q[1]);
        hi[0] = hi[0].max(q[0]);
        hi[1] = hi[1].max(q[1]);
    }
    let size = [hi[0] - lo[0], hi[1] - lo[1]];
    if size.iter().any(|v| !v.is_finite() || *v < 1e-8) {
        return Err(invalid(
            "selected scan region has no two-dimensional support in the chosen flow frame",
        ));
    }
    let scale = (size[0] * size[1]).sqrt();
    if !scale.is_finite() || scale <= 1e-8 {
        return Err(invalid("invalid freeform region extent"));
    }
    let f = Frame {
        origin: region.center,
        x,
        y,
        n,
        lo,
        size,
        scale,
    };
    // A connected oriented mesh with simple, nonoverlapping boundary and positive
    // projected triangles is a single graph chart. Never fit across a folded scan.
    let mut sign = 0.;
    for tri in &region.triangles {
        let [a, b, c] = *tri;
        let (a, b, c) = (
            get(&region.positions, a)?,
            get(&region.positions, b)?,
            get(&region.positions, c)?,
        );
        let cross = (b - a).cross(c - a);
        let length = cross.length();
        let projected = cross.dot(n);
        if !length.is_finite() || length <= 1e-16 || projected.abs() < length * 0.05 {
            return Err(invalid(
                "selected region contains a degenerate or near-vertical face; select a smaller single-flow patch",
            ));
        }
        if sign == 0. {
            sign = projected.signum();
        } else if projected * sign <= 0. {
            return Err(invalid(
                "selected region folds over the fitting plane; split it into separate surface patches",
            ));
        }
    }
    Ok(f)
}
fn accumulate(
    a: &mut RealMatrix,
    b: &mut [f64],
    row: &[f64],
    value: f64,
    weight: f64,
) -> Result<()> {
    if !value.is_finite() || !weight.is_finite() || weight < 0. {
        return Err(invalid("invalid fitting sample"));
    }
    for (i, &x) in row.iter().enumerate() {
        *b.get_mut(i)
            .ok_or_else(|| invalid("invalid fit equation"))? += weight * x * value;
        for (j, &y) in row.iter().enumerate() {
            a.set(i, j, a.get(i, j)? + weight * x * y)?;
        }
    }
    Ok(())
}
fn heights(region: &PreparedRegion, f: Frame, fairing: f64) -> Result<Vec<f64>> {
    if region.samples.len() < 16 || region.samples.len() > 20_000 {
        return Err(invalid(
            "freeform fitting needs 16 to 20000 prepared samples",
        ));
    }
    let total = region.samples.iter().map(|(_, w)| *w).sum::<f64>();
    if !total.is_finite() || total <= 0. {
        return Err(invalid("scan fit samples have no area"));
    }
    let mut a = RealMatrix::new(15, 15, MatrixType::Full, 15)?;
    let mut b = vec![0.; 16];
    for &(p, w) in &region.samples {
        let [u, v, z] = f.coordinates(p);
        accumulate(&mut a, &mut b, &tensor(u, v), z, w / total)?;
    }
    // Physical bending scaled by the geometric mean chart size: uniformly resizing
    // a scan leaves this dimensionless regularizer unchanged. It does not shrink planes.
    for direction in 0..2 {
        for i in 0..2 {
            for j in 0..4 {
                let mut row = vec![0.; 16];
                for (offset, coefficient) in [1., -2., 1.].into_iter().enumerate() {
                    let index = if direction == 0 {
                        (i + offset) * 4 + j
                    } else {
                        j * 4 + i + offset
                    };
                    *row.get_mut(index)
                        .ok_or_else(|| invalid("invalid fairness equation"))? = coefficient;
                }
                let extent = get(&f.size, direction)?;
                let weight = fairing / 16. * (f.scale / extent).powi(4);
                accumulate(&mut a, &mut b, &row, 0., weight)?;
            }
        }
    }
    // Mixed differences control saddle ripples as well as row/column bending.
    for i in 0..3 {
        for j in 0..3 {
            let mut row = vec![0.; 16];
            for (u, v, c) in [
                (i, j, 1.),
                (i + 1, j, -1.),
                (i, j + 1, -1.),
                (i + 1, j + 1, 1.),
            ] {
                *row.get_mut(u * 4 + v)
                    .ok_or_else(|| invalid("invalid mixed fairness equation"))? = c;
            }
            accumulate(&mut a, &mut b, &row, 0., fairing / 8.)?;
        }
    }
    let pivot = a.lu_decompose_pivot().map_err(|_| {
        invalid("scan region does not support a stable bicubic patch in this flow direction")
    })?;
    a.for_back_pivot(&pivot, &mut b)
        .map_err(|_| invalid("freeform least-squares system is singular"))?;
    if b.iter().any(|v| !v.is_finite() || v.abs() > 1e6) {
        return Err(invalid(
            "freeform fit is ill-conditioned; select a smaller, better distributed region",
        ));
    }
    Ok(b)
}
fn height(h: &[f64], u: f64, v: f64) -> Result<f64> {
    if h.len() != 16 {
        return Err(invalid("invalid bicubic control net"));
    }
    Ok(tensor(u, v).iter().zip(h).map(|(a, b)| a * b).sum())
}
fn surface(f: Frame, h: &[f64]) -> Result<Surface> {
    let mut net = Vec::new();
    for i in 0..4 {
        let mut row = Vec::new();
        for j in 0..4 {
            row.push(CPoint::from_point(f.point(
                f64::from(i) / 3.,
                f64::from(j) / 3.,
                get(h, (i * 4 + j) as usize)?,
            )));
        }
        net.push(row);
    }
    let knots = vec![0., 0., 0., 0., 1., 1., 1., 1.];
    let nurbs = NurbSurface::new(net, 3, 3, knots.clone(), knots)?;
    Ok(BSplineSurface::from_gw_nurb(&nurbs).into())
}
/// Blossom of one cubic Bernstein basis at a multiset of three parameter values.
fn blossom(index: usize, args: [f64; 3]) -> f64 {
    let mut sum = 0.;
    for mask in 0_u32..8 {
        if mask.count_ones() as usize != index {
            continue;
        }
        let mut product = 1.;
        for (bit, t) in args.into_iter().enumerate() {
            product *= if mask & (1 << bit) != 0 { t } else { 1. - t };
        }
        sum += product;
    }
    sum
}
fn composed_basis(index: usize, a: f64, b: f64) -> [f64; 4] {
    [
        blossom(index, [a, a, a]),
        blossom(index, [a, a, b]),
        blossom(index, [a, b, b]),
        blossom(index, [b, b, b]),
    ]
}
fn edge(f: Frame, h: &[f64], a: [f64; 2], b: [f64; 2]) -> Result<Curve> {
    let choose3 = [1., 3., 3., 1.];
    let choose6 = [1., 6., 15., 20., 15., 6., 1.];
    let mut z = vec![0.; 7];
    for i in 0..4 {
        for j in 0..4 {
            let u = composed_basis(i, a[0], b[0]);
            let v = composed_basis(j, a[1], b[1]);
            let c = get(h, i * 4 + j)?;
            for (r, &x) in u.iter().enumerate() {
                for (s, &y) in v.iter().enumerate() {
                    let product =
                        c * x * y * get(&choose3, r)? * get(&choose3, s)? / get(&choose6, r + s)?;
                    *z.get_mut(r + s)
                        .ok_or_else(|| invalid("invalid lifted edge coefficient"))? += product;
                }
            }
        }
    }
    let mut points = Vec::new();
    for (i, h) in z.into_iter().enumerate() {
        let t = i as f64 / 6.;
        points.push(CPoint::from_point(f.point(
            a[0] * (1. - t) + b[0] * t,
            a[1] * (1. - t) + b[1] * t,
            h,
        )));
    }
    let curve = NurbCurve::new(
        points,
        6,
        vec![0., 0., 0., 0., 0., 0., 0., 1., 1., 1., 1., 1., 1., 1.],
    )?;
    Ok(BSplineCurve::from_gw_nurb(3, &curve).into())
}
/// Fit and trim one bicubic height patch to the selected scan boundary.
///
/// The caller compares the report's sampled residuals against its requested tolerance.
/// `fairing` is dimensionless in [0,1]; no inter-patch continuity is claimed.
pub fn fit_freeform(
    k: &mut Kernel,
    region: &PreparedRegion,
    flow: Option<[f64; 3]>,
    tolerance: f64,
    fairing: f64,
) -> Result<(Brep, PatchReport)> {
    if !tolerance.is_finite()
        || tolerance <= 0.
        || tolerance > 1e6
        || !fairing.is_finite()
        || !(0.0..=1.).contains(&fairing)
    {
        return Err(invalid(
            "freeform tolerance must be positive and fairing between zero and one",
        ));
    }
    if region.positions.len() > 6_000_000
        || region.triangles.len() > 2_000_000
        || region.loops.iter().map(Vec::len).sum::<usize>() > 4096
    {
        return Err(invalid(
            "freeform region exceeds the supported scan or boundary budget",
        ));
    }
    let f = frame(region, flow)?;
    let h = heights(region, f, fairing)?;
    let mut squared = 0.;
    let mut max: f64 = 0.;
    let mut measured = 0usize;
    let mut measure = |p: Vec3| -> Result<()> {
        let [u, v, z] = f.coordinates(p);
        let error = (height(&h, u, v)? - z).abs() * f.scale;
        if !error.is_finite() {
            return Err(invalid("non-finite freeform fitting residual"));
        }
        squared += error * error;
        max = max.max(error);
        measured += 1;
        Ok(())
    };
    for &p in &region.positions {
        measure(p)?;
    }
    for tri in &region.triangles {
        let [a, b, c] = *tri;
        measure(
            (get(&region.positions, a)? + get(&region.positions, b)? + get(&region.positions, c)?)
                / 3.,
        )?;
    }
    if measured == 0 {
        return Err(invalid("freeform fit has no measurement samples"));
    }
    let mut loops = Vec::new();
    for boundary in &region.loops {
        let mut uv = Vec::new();
        for &index in boundary {
            let [u, v, _] = f.coordinates(get(&region.positions, index)?);
            uv.push([u, v]);
        }
        loops.push(uv);
    }
    let loops = ordered_uv_loops(loops)?;
    let mut edges = Vec::new();
    for boundary in &loops {
        for (a, b) in boundary
            .iter()
            .zip(boundary.iter().cycle().skip(1))
            .take(boundary.len())
        {
            edges.push(edge(f, &h, *a, *b)?);
        }
    }
    let count = edges.len();
    let geometric_tolerance = (f.scale * 1e-10).max(1e-8).min(tolerance * 0.01);
    let body = trimmed_sheet(k, surface(f, &h)?, &loops, geometric_tolerance, Some(edges))?;
    Ok((
        body,
        PatchReport {
            fit_samples: region.samples.len(),
            measured_samples: measured,
            rms: (squared / measured as f64).sqrt(),
            max,
            flow: array(f.x),
            normal: array(f.n),
            control_points: [4, 4],
            boundary_loops: loops.len(),
            boundary_edges: count,
            fairing,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paraboloid(scale: f64) -> PreparedRegion {
        let mut positions = Vec::new();
        let mut triangles = Vec::new();
        for i in 0..9 {
            for j in 0..9 {
                let (x, y) = (f64::from(i), f64::from(j));
                positions.push(Vec3::new(x, y, 0.03 * x * x + 0.01 * y * y + 0.02 * x * y) * scale);
            }
        }
        for i in 0..8 {
            for j in 0..8 {
                let a = i * 9 + j;
                triangles.extend([[a, a + 9, a + 10], [a, a + 10, a + 1]]);
            }
        }
        let samples = positions.iter().map(|p| (*p, 1.)).collect();
        PreparedRegion {
            positions,
            triangles,
            loops: vec![vec![0, 72, 80, 8]],
            samples,
            center: Vec3::ZERO,
            normal: Vec3::Z,
            x: Vec3::X,
            y: Vec3::Y,
            face_normals: Vec::new(),
        }
    }
    #[test]
    fn bounded_freeform_sheet_keeps_a_real_hole_and_compact_control_net() {
        let mut region = paraboloid(1.);
        region.triangles.retain(|tri| {
            let p = region.positions[tri[0]];
            !(p.x >= 3. && p.x < 5. && p.y >= 3. && p.y < 5.)
        });
        region.loops.push(vec![30, 32, 50, 48]);
        let mut kernel = Kernel::default();
        let (body, report) = fit_freeform(&mut kernel, &region, None, 0.5, 0.).unwrap();
        let faces = crate::topology::brep::faces(&body, None).unwrap();
        assert_eq!(faces.len(), 1);
        assert_eq!(
            crate::topology::face::loops(&body, faces[0], false, None)
                .unwrap()
                .len(),
            2
        );
        assert_eq!(body.edges().len(), 8);
        assert_eq!(report.control_points, [4, 4]);
        // The mesh's linear triangle centroids differ slightly from the analytic
        // polynomial through its vertices; the diagnostic measures those too.
        assert!(report.max < 0.02);
        assert!(report.measured_samples > region.positions.len());
    }
    #[test]
    fn bicubic_scan_fit_recovers_a_polynomial_in_a_rotated_flow_frame() {
        let region = paraboloid(1.);
        let f = frame(&region, Some([1., 1., 0.])).unwrap();
        let h = heights(&region, f, 0.).unwrap();
        for &p in &region.positions {
            let [u, v, z] = f.coordinates(p);
            assert!((height(&h, u, v).unwrap() - z).abs() * f.scale < 1e-7);
        }
        assert!(f.x.dot(Vec3::new(1., 1., 0.).unitize().unwrap()) > 1. - 1e-12);
    }
    #[test]
    fn freeform_fairing_is_scale_independent_and_preserves_planes() {
        let a = paraboloid(1.);
        let b = paraboloid(100.);
        let fa = frame(&a, None).unwrap();
        let fb = frame(&b, None).unwrap();
        let ha = heights(&a, fa, 0.01).unwrap();
        let hb = heights(&b, fb, 0.01).unwrap();
        for (a, b) in ha.iter().zip(hb) {
            assert!((a - b).abs() < 1e-9);
        }
        let mut plane = paraboloid(1.);
        for p in &mut plane.positions {
            p.z = 1. + 0.2 * p.x - 0.3 * p.y;
        }
        plane.samples = plane.positions.iter().map(|p| (*p, 1.)).collect();
        let f = frame(&plane, None).unwrap();
        let h = heights(&plane, f, 1.).unwrap();
        for p in plane.positions {
            let [u, v, z] = f.coordinates(p);
            assert!((height(&h, u, v).unwrap() - z).abs() < 1e-9);
        }
    }
    #[test]
    fn straight_uv_trim_is_lifted_exactly_to_degree_six() {
        let r = paraboloid(20.);
        let f = frame(&r, None).unwrap();
        let h = heights(&r, f, 0.).unwrap();
        let k = Kernel::default();
        for (a, b) in [
            ([0., 0.], [1., 1.]),
            ([0.1, 0.9], [0.8, 0.2]),
            ([0., 0.], [0., 1.]),
        ] {
            let c = edge(f, &h, a, b).unwrap();
            for i in 0..21 {
                let t = f64::from(i) / 20.;
                let u = a[0] * (1. - t) + b[0] * t;
                let v = a[1] * (1. - t) + b[1] * t;
                let expected = f.point(u, v, height(&h, u, v).unwrap());
                assert!((c.evaluate_point(&k, t).unwrap() - expected).length() < 1e-8);
            }
        }
    }
    #[test]
    fn folded_scan_and_normal_flow_are_refused() {
        let mut r = paraboloid(1.);
        assert!(frame(&r, Some([0., 0., 1.])).is_err());
        r.triangles[0].swap(1, 2);
        assert!(frame(&r, None).is_err());
    }
}
