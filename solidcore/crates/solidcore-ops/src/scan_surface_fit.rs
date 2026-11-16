//! Bounded scan-surface reconstruction. Numerical fitting and trim geometry live in core.
use crate::body::{finish, Body, Expect};
use crate::session::run;
use crate::{Error, Result};
pub use solidcore::query::scan_surface_fit::{Kind, Options, Report};
use solidcore::SolidError;
use solidcore_record::brep::Brep as Record;

/// Fit one connected selected region and return an open, trimmed sheet plus sampled errors.
pub fn fit(
    positions: &[[f64; 3]],
    triangles: &[[u32; 3]],
    options: &Options,
) -> Result<(Record, Report)> {
    let (body, report) = run("scan_surface_fit::fit", |k| {
        let (body, report) =
            match solidcore::query::scan_surface_fit::fit(k, positions, triangles, options) {
                Ok(value) => value,
                Err(SolidError::InvalidInput { reason, .. }) => {
                    return Ok(Err(Error::Unsupported(reason)));
                }
                Err(error) => return Err(error),
            };
        Ok(finish(k, body, Expect::FACES)?.map(|body| (body, report)))
    })??;
    let (record, _) = Body::new(body).to_record()?;
    Ok((record, report))
}
