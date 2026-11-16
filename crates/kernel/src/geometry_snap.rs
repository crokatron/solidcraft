//! Exact topology queries for Move handle placement. Reuses the cached kernel body.
use crate::{Body, KernelError, Result};
pub use solidcore_ops::geometry_snap::{Sample, align_axis, frame_normal, line_intersection};
pub use solidcore_ops::measure::point_distance;
impl Body {
    pub fn handle_edge_sample(&self, index: usize, point: [f64; 3], midpoint: bool) -> Result<solidcore_ops::geometry_snap::Sample> {
        let (b, o) = self.ops_body().ok_or_else(|| KernelError::Invalid("Exact edge snapping requires a BRep body".into()))?;
        b.handle_edge_sample(o, index, point, midpoint).map_err(|e| KernelError::Invalid(e.to_string()))
    }
    pub fn handle_face_axis_center(&self, index: usize, frame: &solidcore_record::brep::Frame) -> Result<solidcore_ops::geometry_snap::Sample> {
        let (b, o) = self.ops_body().ok_or_else(|| KernelError::Invalid("Exact face snapping requires a BRep body".into()))?;
        b.handle_face_axis_center(o, index, frame.origin, frame.x_axis, frame.y_axis).map_err(|e| KernelError::Invalid(e.to_string()))
    }
    pub fn handle_face_sample(&self, index: usize, point: [f64; 3]) -> Result<solidcore_ops::geometry_snap::Sample> {
        let (b, o) = self.ops_body().ok_or_else(|| KernelError::Invalid("Exact face snapping requires a BRep body".into()))?;
        b.handle_face_sample(o, index, point).map_err(|e| KernelError::Invalid(e.to_string()))
    }
}
