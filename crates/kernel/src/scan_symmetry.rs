//! Mirror-plane operations through the independent kernel boundary.
use crate::{KernelError, Result};
pub use solidcore_ops::scan_symmetry::{Alignment, PairFit, Plane};
/// Validate a theoretical datum and normalize its normal.
pub fn explicit_plane(origin: [f64; 3], normal: [f64; 3]) -> Result<Plane> {
    solidcore_ops::scan_symmetry::explicit_plane(origin, normal).map_err(|e| KernelError::Failed(e.to_string()))
}
/// Fit a theoretical mirror plane to 3–4096 corresponding landmark pairs.
pub fn fit_pairs(pairs: &[[[f64; 3]; 2]]) -> Result<PairFit> {
    solidcore_ops::scan_symmetry::fit_pairs(pairs).map_err(|e| KernelError::Failed(e.to_string()))
}
/// Align rigidly to target 0=YZ, 1=XZ, 2=XY, with flip, clocking and in-plane offset.
pub fn align_to_csys(plane: Plane, target: usize, flip: bool, clock_deg: f64, offset: [f64; 2]) -> Result<Alignment> {
    solidcore_ops::scan_symmetry::align_to_csys(plane, target, flip, clock_deg, offset).map_err(|e| KernelError::Failed(e.to_string()))
}
