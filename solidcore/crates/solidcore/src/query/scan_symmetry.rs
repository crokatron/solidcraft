//! Reproducible mirror-plane construction from explicit input or paired landmarks.
//!
//! Original least-squares derivation: for midpoint m and half-difference h,
//! |reflect(a)-b|²/4 = |h|² + (n·(m-o))² - (n·h)².
//! The least eigenvector of midpoint covariance minus half-difference covariance
//! therefore gives the global paired-landmark minimum. This is not automatic scan
//! symmetry detection. At most 4096 pairs; no recursion or threads.
use crate::geom::matrix::Matrix;
use crate::{Result, SolidError, Vec3};
use serde::Serialize;

#[derive(Clone, Copy, Debug, Serialize)]
/// A theoretical plane, with a unit normal when returned by these operations.
pub struct Plane {
    /// A point on the plane in model coordinates.
    pub origin: [f64; 3],
    /// Oriented plane normal.
    pub normal: [f64; 3],
}
#[derive(Clone, Debug, Serialize)]
/// Proper rigid placement of the theoretical plane onto a world coordinate plane.
pub struct Alignment {
    /// Row-major homogeneous rigid matrix acting on column vectors.
    pub transform: [[f64; 4]; 4],
    /// Aligned plane origin, including the requested in-plane offset.
    pub pivot: [f64; 3],
    /// Signed target normal, also the clocking axis.
    pub axis: [f64; 3],
    /// Residual angle between transformed normal and signed target normal.
    pub angular_error_deg: f64,
    /// Absolute transformed-origin distance from the target plane.
    pub distance_error_mm: f64,
}
#[derive(Clone, Debug, Serialize)]
/// Least-squares diagnostics for chosen pairs only; no whole-scan symmetry claim.
pub struct PairFit {
    /// Fitted mirror plane; normal sign is deterministic.
    pub plane: Plane,
    /// Number of supplied corresponding pairs.
    pub pairs: usize,
    /// Root-mean-square distance from reflected left landmarks to right landmarks.
    pub rms_mm: f64,
    /// Largest reflected landmark-pair distance.
    pub max_mm: f64,
}
fn bad(s: &'static str) -> SolidError {
    SolidError::invalid_input("scan_symmetry", s)
}
fn v([x, y, z]: [f64; 3]) -> Result<Vec3> {
    if ![x, y, z].iter().all(|c| c.is_finite() && c.abs() <= 1e12) {
        return Err(bad(
            "coordinates must be finite and within 1e12 model units",
        ));
    }
    Ok(Vec3::new(x, y, z))
}
fn a(p: Vec3) -> [f64; 3] {
    [p.x, p.y, p.z]
}
/// Validate an explicit datum and normalize its nonzero normal.
pub fn explicit_plane(origin: [f64; 3], normal: [f64; 3]) -> Result<Plane> {
    let origin = v(origin)?;
    let normal = v(normal)?;
    if normal.length() <= 1e-12 {
        return Err(bad("mirror plane normal must be nonzero"));
    }
    Ok(Plane {
        origin: a(origin),
        normal: a(normal.unitize()?),
    })
}
/// Reflect a point across a validated theoretical plane.
pub fn reflect_point(plane: Plane, point: [f64; 3]) -> Result<[f64; 3]> {
    let plane = explicit_plane(plane.origin, plane.normal)?;
    let p = v(point)?;
    let o = v(plane.origin)?;
    let n = v(plane.normal)?;
    Ok(a(p - n * (2. * n.dot(p - o))))
}
/// Fit 3–4096 pairs with non-collinear midpoints; reject an ambiguous minimizer.
pub fn fit_pairs(pairs: &[[[f64; 3]; 2]]) -> Result<PairFit> {
    if pairs.len() < 3 || pairs.len() > 4096 {
        return Err(bad(
            "provide between 3 and 4096 corresponding landmark pairs",
        ));
    }
    let mut center = Vec3::ZERO;
    let mut data = Vec::new();
    for &[left, right] in pairs {
        let left = v(left)?;
        let right = v(right)?;
        let mid = (left + right) * 0.5;
        let half = (right - left) * 0.5;
        center = center + mid / (pairs.len() as f64);
        data.push((mid, half));
    }
    let scale = data
        .iter()
        .map(|&(m, h)| (m - center).length().max(h.length()))
        .fold(0., f64::max);
    if scale <= 1e-12 {
        return Err(bad("landmark pairs do not define a mirror plane"));
    }
    let longest = data
        .iter()
        .map(|&(m, _)| (m - center) / scale)
        .max_by(|a, b| a.length_squared().total_cmp(&b.length_squared()))
        .ok_or_else(|| bad("missing landmarks"))?;
    if data
        .iter()
        .map(|&(m, _)| longest.cross((m - center) / scale).length())
        .fold(0., f64::max)
        <= 1e-8
    {
        return Err(bad(
            "pair midpoints must span a plane; choose three separated, non-collinear pairs",
        ));
    }
    let mut matrix = Matrix::new(3, 3);
    for &(m, h) in &data {
        let d = (m - center) / scale;
        let h = h / scale;
        for r in 0..3 {
            for c in 0..3 {
                matrix.set_at(
                    r,
                    c,
                    matrix.at(r, c)?
                        + (d.component(r) * d.component(c) - h.component(r) * h.component(c))
                            / (pairs.len() as f64),
                )?;
            }
        }
    }
    let (values, axes) = matrix.compute_3x3_eigen_vectors()?;
    let first = *values.first().ok_or_else(|| bad("missing eigenvalue"))?;
    let second = *values.get(1).ok_or_else(|| bad("missing eigenvalue"))?;
    if second - first <= 1e-10 {
        return Err(bad(
            "paired landmarks leave the symmetry plane ambiguous; choose more separated pairs",
        ));
    }
    let mut normal = axes
        .first()
        .ok_or_else(|| bad("missing eigenvector"))?
        .unitize()?;
    // Deterministic orientation independent of pair ordering and endpoint ordering.
    let largest = if normal.x.abs() >= normal.y.abs() && normal.x.abs() >= normal.z.abs() {
        normal.x
    } else if normal.y.abs() >= normal.z.abs() {
        normal.y
    } else {
        normal.z
    };
    if largest < 0. {
        normal = -normal;
    }
    let plane = Plane {
        origin: a(center),
        normal: a(normal),
    };
    let mut squared = 0.;
    let mut max_mm: f64 = 0.;
    for &[left, right] in pairs {
        let error = (v(reflect_point(plane, left)?)? - v(right)?).length();
        squared += error * error / (pairs.len() as f64);
        max_mm = max_mm.max(error);
    }
    Ok(PairFit {
        plane,
        pairs: pairs.len(),
        rms_mm: squared.sqrt(),
        max_mm,
    })
}
/// Rigid, row-major transform. Target 0=YZ (+X), 1=XZ (+Y), 2=XY (+Z).
/// Retains the source origin's tangential location, then applies clocking and offset.
/// No reflection is applied to the scan; flip changes the target plane orientation.
pub fn align_to_csys(
    plane: Plane,
    target: usize,
    flip: bool,
    clock_deg: f64,
    offset: [f64; 2],
) -> Result<Alignment> {
    let plane = explicit_plane(plane.origin, plane.normal)?;
    if !clock_deg.is_finite() || !offset.iter().all(|x| x.is_finite() && x.abs() <= 1e12) {
        return Err(bad("clocking and offsets must be finite"));
    }
    let [u, w] = offset;
    let (axis, shift) = match target {
        0 => (Vec3::new(1., 0., 0.), Vec3::new(0., u, w)),
        1 => (Vec3::new(0., 1., 0.), Vec3::new(u, 0., w)),
        2 => (Vec3::new(0., 0., 1.), Vec3::new(u, w, 0.)),
        _ => return Err(bad("target axis must be 0, 1 or 2")),
    };
    let axis = if flip { -axis } else { axis };
    let n = v(plane.normal)?;
    let o = v(plane.origin)?;
    let cross = n.cross(axis);
    let sin = cross.length();
    let cos = n.dot(axis).clamp(-1., 1.);
    let (rot, angle) = if sin > 1e-12 {
        (cross / sin, sin.atan2(cos))
    } else if cos < 0. {
        let seed = if n.x.abs() < 0.8 {
            Vec3::new(1., 0., 0.)
        } else {
            Vec3::new(0., 1., 0.)
        };
        (n.cross(seed).unitize()?, std::f64::consts::PI)
    } else {
        (axis, 0.)
    };
    let turn = |p: Vec3, r: Vec3, t: f64| {
        p * t.cos() + r.cross(p) * t.sin() + r * (r.dot(p) * (1. - t.cos()))
    };
    let rotate = |p: Vec3| turn(turn(p, rot, angle), axis, (clock_deg % 360.).to_radians());
    let pivot = o - axis * axis.dot(o) + shift;
    let t = pivot - rotate(o);
    let x = rotate(Vec3::new(1., 0., 0.));
    let y = rotate(Vec3::new(0., 1., 0.));
    let z = rotate(Vec3::new(0., 0., 1.));
    let angular_error_deg = rotate(n)
        .cross(axis)
        .length()
        .atan2(rotate(n).dot(axis))
        .to_degrees()
        .abs();
    Ok(Alignment {
        transform: [
            [x.x, y.x, z.x, t.x],
            [x.y, y.y, z.y, t.y],
            [x.z, y.z, z.z, t.z],
            [0., 0., 0., 1.],
        ],
        pivot: a(pivot),
        axis: a(axis),
        angular_error_deg,
        distance_error_mm: axis.dot(rotate(o) + t).abs(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paired_landmarks_recover_oblique_plane_and_residuals() {
        let plane = explicit_plane([12., -5., 8.], [1., 2., 3.]).unwrap();
        let left = [
            [31., 5., -8.],
            [-4., 12., 16.],
            [5., -28., 7.],
            [20., 14., 30.],
        ];
        let pairs = left.map(|p| [p, reflect_point(plane, p).unwrap()]);
        let fit = fit_pairs(&pairs).unwrap();
        assert!(fit.rms_mm < 1e-8 && fit.max_mm < 1e-8);
        assert!(
            v(fit.plane.normal)
                .unwrap()
                .cross(v(plane.normal).unwrap())
                .length()
                < 1e-9
        );
        let mut noisy = pairs;
        noisy[0][1][0] += 0.4;
        let fit = fit_pairs(&noisy).unwrap();
        assert!(fit.rms_mm > 0.01 && fit.max_mm < 0.5);
    }
    #[test]
    fn alignment_is_rigid_for_all_targets_flip_clock_and_antiparallel() {
        for normal in [[1., 2., 3.], [0., 0., -1.], [0., 1., 0.]] {
            for target in 0..3 {
                for flip in [false, true] {
                    let plane = explicit_plane([12., -5., 8.], normal).unwrap();
                    let result = align_to_csys(plane, target, flip, 33., [4., -7.]).unwrap();
                    assert!(result.angular_error_deg < 1e-10 && result.distance_error_mm < 1e-10);
                    let m = result.transform;
                    let cols = [
                        Vec3::new(m[0][0], m[1][0], m[2][0]),
                        Vec3::new(m[0][1], m[1][1], m[2][1]),
                        Vec3::new(m[0][2], m[1][2], m[2][2]),
                    ];
                    assert!((cols[0].cross(cols[1]).dot(cols[2]) - 1.).abs() < 1e-12);
                    assert!(
                        (cols[0].length() - 1.).abs() < 1e-12 && cols[0].dot(cols[1]).abs() < 1e-12
                    );
                }
            }
        }
    }
    #[test]
    fn ambiguous_collinear_and_hostile_input_refused() {
        assert!(fit_pairs(&[]).is_err());
        assert!(
            fit_pairs(&[
                [[-1., 0., 0.], [1., 0., 0.]],
                [[-1., 1., 0.], [1., 1., 0.]],
                [[-1., 2., 0.], [1., 2., 0.]]
            ])
            .is_err()
        );
        assert!(explicit_plane([f64::NAN, 0., 0.], [1., 0., 0.]).is_err());
        assert!(explicit_plane([0.; 3], [0.; 3]).is_err());
        let p = explicit_plane([0.; 3], [0., 0., 1.]).unwrap();
        assert!(align_to_csys(p, 3, false, 0., [0.; 2]).is_err());
        assert!(align_to_csys(p, 2, false, f64::NAN, [0.; 2]).is_err());
    }
}
