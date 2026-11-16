//! Reproducible manual mirror-plane workflow; all geometry is computed in core.
use crate::{Error, Result};
use solidcore::query::scan_symmetry as core;
pub use solidcore::query::scan_symmetry::{Alignment, PairFit, Plane};
/// Validate a theoretical datum and normalize its normal.
pub fn explicit_plane(origin: [f64; 3], normal: [f64; 3]) -> Result<Plane> {
    core::explicit_plane(origin, normal).map_err(|e| match e {
        solidcore::SolidError::InvalidInput { reason, .. } => Error::Unsupported(reason),
        _ => Error::Argument,
    })
}
/// Fit a theoretical mirror plane to 3–4096 corresponding landmark pairs.
pub fn fit_pairs(pairs: &[[[f64; 3]; 2]]) -> Result<PairFit> {
    core::fit_pairs(pairs).map_err(|e| match e {
        solidcore::SolidError::InvalidInput { reason, .. } => Error::Unsupported(reason),
        _ => Error::Argument,
    })
}
/// Align rigidly to target 0=YZ, 1=XZ, 2=XY, with flip, clocking and in-plane offset.
pub fn align_to_csys(
    plane: Plane,
    target: usize,
    flip: bool,
    clock_deg: f64,
    offset: [f64; 2],
) -> Result<Alignment> {
    core::align_to_csys(plane, target, flip, clock_deg, offset).map_err(|e| match e {
        solidcore::SolidError::InvalidInput { reason, .. } => Error::Unsupported(reason),
        _ => Error::Argument,
    })
}
