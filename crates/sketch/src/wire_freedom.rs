//! Local spatial mobility from the complete constraint Jacobian, including rigid motion.
use super::{WireConstraint, residual};
use crate::Sketch;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct SpatialPointFreedom {
    pub wire: String,
    pub index: usize,
    pub free_axes: [bool; 3],
    /// Orthonormal basis of locally admissible point velocities, in the wire frame.
    pub directions: Vec<[f64; 3]>,
}
#[derive(Clone, Debug, Serialize)]
pub struct SpatialFreedom {
    pub dof: Option<usize>,
    pub variables: usize,
    pub rank: usize,
    pub points: Vec<SpatialPointFreedom>,
    pub reason: Option<String>,
    pub status: String,
}
impl Sketch {
    /// Instantaneous DOF, not a promise of a finite motion at a singular configuration.
    /// The UI caches this bounded analysis by sketch revision rather than calling per frame.
    pub fn wire_freedom(&self) -> SpatialFreedom {
        let points: Vec<_> =
            self.wires.iter().enumerate().filter(|(_, w)| w.link.is_none()).flat_map(|(w, wire)| (0..wire.fit.len()).map(move |p| (w, p))).collect();
        let n = points.len().saturating_mul(3);
        let mut answer = SpatialFreedom { dof: None, variables: n, rank: 0, points: Vec::new(), reason: None, status: "unknown".into() };
        if points.len() > 96 {
            answer.reason = Some("Spatial freedom analysis supports up to 96 editable fit points".into());
            return answer;
        }
        if let Err(error) = self.validate_wire_constraints() {
            answer.reason = Some(error.to_string());
            return answer;
        }
        let relations: Vec<&WireConstraint> = self.wire_constraints.iter().collect();
        let Some(r) = residual(&self.wires, &relations) else {
            answer.reason = Some("Degenerate spatial geometry".into());
            return answer;
        };
        if r.iter().any(|v| v.abs() > 1e-6) {
            answer.status = "conflicting".into();
            answer.reason = Some("Spatial constraints are not satisfied".into());
            return answer;
        }
        let mut wires = self.wires.clone();
        let mut rows = vec![vec![0.; n]; r.len()];
        for (pi, &(wi, pt)) in points.iter().enumerate() {
            for axis in 0..3 {
                let at = wires[wi].fit[pt];
                let x = [at.x, at.y, at.z][axis];
                let h = 1e-4_f64.max(x.abs() * 1e-8);
                let assign = |wires: &mut [crate::Wire], v: f64| {
                    let p = &mut wires[wi].fit[pt];
                    match axis {
                        0 => p.x = v,
                        1 => p.y = v,
                        _ => p.z = v,
                    }
                };
                assign(&mut wires, x + h);
                let plus = residual(&wires, &relations);
                assign(&mut wires, x - h);
                let minus = residual(&wires, &relations);
                assign(&mut wires, x);
                let (Some(plus), Some(minus)) = (plus, minus) else {
                    answer.reason = Some("Spatial Jacobian is undefined at this geometry".into());
                    return answer;
                };
                for i in 0..r.len() {
                    rows[i][pi * 3 + axis] = (plus[i] - minus[i]) / (2. * h);
                }
            }
        }
        let jacobian = rows.clone();
        // Normalize equation rows before pivoted modified Gram-Schmidt so angular and
        // length equations use the same relative rank test. Reorthogonalization keeps
        // redundant coincidence/tangency equations from being counted twice.
        rows.retain_mut(|row| {
            let norm = dot(row, row).sqrt();
            if norm < 1e-10 {
                return false;
            }
            for v in row {
                *v /= norm;
            }
            true
        });
        let mut basis: Vec<Vec<f64>> = Vec::new();
        while !rows.is_empty() {
            let Some((best, norm)) = rows.iter().enumerate().map(|(i, r)| (i, dot(r, r).sqrt())).max_by(|a, b| a.1.total_cmp(&b.1)) else {
                break;
            };
            if norm < 1e-6 {
                break;
            }
            let mut q = rows.swap_remove(best);
            for x in &mut q {
                *x /= norm;
            }
            for row in &mut rows {
                for _ in 0..2 {
                    let d = dot(row, &q);
                    for (a, b) in row.iter_mut().zip(&q) {
                        *a -= d * b;
                    }
                }
            }
            basis.push(q);
            if basis.len() == n {
                break;
            }
        }
        answer.rank = basis.len();
        if n > basis.len() && r.len() > basis.len() && second_order_obstruction(&self.wires, &points, &relations, &r, &jacobian, &basis) {
            answer.status = "singular".into();
            answer.reason = Some("Singular spatial configuration: linearized freedom does not establish a finite movement. Adjust the geometry or dimensions before relying on movement indicators.".into());
            return answer;
        }
        answer.dof = Some(n.saturating_sub(basis.len()));
        answer.status = if answer.dof == Some(0) { "fully_constrained" } else { "under_constrained" }.into();
        for (pi, &(wi, index)) in points.iter().enumerate() {
            let mut block = [[0.; 3]; 3];
            for (a, row) in block.iter_mut().enumerate() {
                for (b, v) in row.iter_mut().enumerate() {
                    *v = if a == b { 1. } else { 0. };
                    for q in &basis {
                        *v -= q[pi * 3 + a] * q[pi * 3 + b];
                    }
                }
            }
            let free_axes = std::array::from_fn(|a| block[a][a] > 1e-7);
            let mut directions: Vec<[f64; 3]> = Vec::new();
            for col in 0..3 {
                let mut q = [block[0][col], block[1][col], block[2][col]];
                for _ in 0..2 {
                    for b in &directions {
                        let d = dot(&q, b);
                        for i in 0..3 {
                            q[i] -= d * b[i];
                        }
                    }
                }
                let norm = dot(&q, &q).sqrt();
                if norm > 1e-7 {
                    for v in &mut q {
                        *v /= norm;
                    }
                    directions.push(q);
                }
            }
            answer.points.push(SpatialPointFreedom { wire: self.wires[wi].id.clone(), index, free_axes, directions });
        }
        answer
    }
}
fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}

/// At a singular point, J*v=0 need not extend to a finite movement. The second
/// derivative must lie in range(J), so that a second-order correction can cancel
/// it. Two symmetric step sizes distinguish quadratic obstruction from ordinary
/// higher-order truncation. This bounded check refuses a misleading freedom count;
/// it does not claim to classify every possible higher-order singularity.
fn second_order_obstruction(
    wires: &[crate::Wire],
    points: &[(usize, usize)],
    relations: &[&WireConstraint],
    residual_at: &[f64],
    jacobian: &[Vec<f64>],
    row_basis: &[Vec<f64>],
) -> bool {
    let n = points.len() * 3;
    let m = residual_at.len();
    if n == 0 || m == 0 {
        return false;
    }
    let scales: Vec<f64> = jacobian
        .iter()
        .map(|row| {
            let norm = dot(row, row).sqrt();
            if norm > 1e-10 { 1. / norm } else { 1. }
        })
        .collect();
    let mut columns: Vec<Vec<f64>> = (0..n).map(|col| (0..m).map(|row| jacobian[row][col] * scales[row]).collect()).collect();
    let mut range: Vec<Vec<f64>> = Vec::new();
    while !columns.is_empty() {
        let Some((best, norm)) = columns.iter().enumerate().map(|(i, q)| (i, dot(q, q).sqrt())).max_by(|a, b| a.1.total_cmp(&b.1)) else {
            break;
        };
        if norm < 1e-6 {
            break;
        }
        let mut q = columns.swap_remove(best);
        for v in &mut q {
            *v /= norm;
        }
        for col in &mut columns {
            for _ in 0..2 {
                let d = dot(col, &q);
                for (a, b) in col.iter_mut().zip(&q) {
                    *a -= d * b;
                }
            }
        }
        range.push(q);
    }
    // With independent equations there is no left-nullspace obstruction to test.
    if range.len() >= m {
        return false;
    }
    let extent = wires
        .iter()
        .filter_map(|w| {
            w.fit.first().map(|a| {
                w.fit
                    .iter()
                    .map(|b| {
                        let d = *b - *a;
                        (d.x * d.x + d.y * d.y + d.z * d.z).sqrt()
                    })
                    .fold(0., f64::max)
            })
        })
        .fold(0., f64::max);
    let h = (extent * 1e-3).clamp(1e-5, 1.);
    let mut shifted = wires.to_vec();
    let mut curvature = |v: &[f64], h: f64| -> Option<Vec<f64>> {
        let mut values = Vec::new();
        for sign in [-1., 1.] {
            for (pi, &(wi, index)) in points.iter().enumerate() {
                let original = wires[wi].fit[index];
                let p = &mut shifted[wi].fit[index];
                p.x = original.x + sign * h * v[3 * pi];
                p.y = original.y + sign * h * v[3 * pi + 1];
                p.z = original.z + sign * h * v[3 * pi + 2];
            }
            values.push(residual(&shifted, relations)?);
        }
        let mut k: Vec<_> = (0..m).map(|i| ((values[0][i] + values[1][i]) * 0.5 - residual_at[i]) * scales[i]).collect();
        for _ in 0..2 {
            for q in &range {
                let d = dot(&k, q);
                for (a, b) in k.iter_mut().zip(q) {
                    *a -= d * b;
                }
            }
        }
        Some(k)
    };
    for col in 0..n {
        let mut v = vec![0.; n];
        v[col] = 1.;
        for q in row_basis {
            for (i, x) in v.iter_mut().enumerate() {
                *x -= q[col] * q[i];
            }
        }
        let norm = dot(&v, &v).sqrt();
        if norm < 1e-7 {
            continue;
        }
        for x in &mut v {
            *x /= norm;
        }
        let (Some(big), Some(small)) = (curvature(&v, h), curvature(&v, h * 0.5)) else {
            continue;
        };
        let norm = dot(&big, &big).sqrt();
        if norm > 1e-8 && dot(&small, &small).sqrt() > 1e-9 {
            let error = big.iter().zip(&small).map(|(a, b)| (a - 4. * b).powi(2)).sum::<f64>().sqrt();
            if error < norm * 0.2 {
                return true;
            }
        }
    }
    false
}
