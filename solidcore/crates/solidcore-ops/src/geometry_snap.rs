//! Read-only exact Move handle samples, rigid axis orientation and line intersections.
use crate::session::{Answer, with_body};
use crate::{Body, Error, Order, Result};
use solidcore::query::geometry_snap as core;
pub use solidcore::query::geometry_snap::Sample;
impl Answer for Sample {}
impl Body {
    /// Exact nearest point or half arc-length point, and its unit tangent.
    ///
    /// # Errors
    /// Invalid topology, invalid input, projection failure or a nonconverging length solve.
    pub fn handle_edge_sample(
        &self,
        order: &Order,
        edge: usize,
        point: [f64; 3],
        midpoint: bool,
    ) -> Result<Sample> {
        let e = crate::order::edge(order, edge)?;
        with_body("handle_edge_sample", self, |k, b| {
            core::edge_sample(k, b, e, point, midpoint)
        })
    }
    /// Centre of the bounded axial span of an analytic cylindrical/conical face.
    ///
    /// # Errors
    /// Invalid topology/frame or boundary bounds cannot be evaluated exactly.
    pub fn handle_face_axis_center(
        &self,
        order: &Order,
        face: usize,
        origin: [f64; 3],
        x: [f64; 3],
        y: [f64; 3],
    ) -> Result<Sample> {
        let f = crate::order::face(order, face)?;
        with_body("handle_face_axis_center", self, |k, b| {
            core::face_axis_center(k, b, f, origin, x, y)
        })
    }
    /// Exact trimmed-face closest point and outward normal at that point.
    ///
    /// # Errors
    /// Invalid topology, failed projection, singular normal or an indeterminate outward side.
    pub fn handle_face_sample(
        &self,
        order: &Order,
        face: usize,
        point: [f64; 3],
    ) -> Result<Sample> {
        let f = crate::order::face(order, face)?;
        with_body("handle_face_sample", self, |k, b| {
            let fu = match crate::measure::outer_faceuse(b, f)? {
                Ok(fu) => fu,
                Err(e) => return Ok(Err(e)),
            };
            core::face_sample(k, b, f, fu, point).map(Ok)
        })?
    }
}
fn error(e: solidcore::SolidError) -> Error {
    match e {
        solidcore::SolidError::InvalidInput { reason, .. } => Error::Unsupported(reason),
        _ => Error::Argument,
    }
}
/// Intersect two extended straight lines in 3D; refuse skew or parallel lines.
///
/// # Errors
/// Invalid coordinates or tolerance, degenerate, parallel or skew lines.
pub fn line_intersection(a: [[f64; 3]; 2], b: [[f64; 3]; 2], tolerance: f64) -> Result<Sample> {
    core::line_intersection(a, b, tolerance).map_err(error)
}
/// Unit normal of the exact frame's defining X and Y axes.
///
/// # Errors
/// Invalid or collinear axes.
pub fn frame_normal(x: [f64; 3], y: [f64; 3]) -> Result<[f64; 3]> {
    core::frame_normal(x, y).map_err(error)
}
/// Rotate a rigid frame to align one axis, preserving its origin and deterministic roll.
///
/// # Errors
/// Invalid axis, nonrigid frame, invalid or zero target direction.
pub fn align_axis(
    frame: &[[f64; 4]; 4],
    axis: usize,
    direction: [f64; 3],
    flip: bool,
) -> Result<[[f64; 4]; 4]> {
    core::align_axis(frame, axis, direction, flip).map_err(error)
}
