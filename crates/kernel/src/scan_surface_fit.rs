//! Scan surface fitting through the independent kernel operations boundary.
use crate::{Body, KernelError, Result};
pub use solidcore_ops::scan_surface_fit::{Kind, Options, Report};

pub fn fit(positions: &[[f64; 3]], triangles: &[[u32; 3]], options: &Options) -> Result<(Body, Report)> {
    let (record, report) = solidcore_ops::scan_surface_fit::fit(positions, triangles, options).map_err(|e| KernelError::Failed(e.to_string()))?;
    Ok((Body::from_record(record)?, report))
}
