//! Read-only queries on B-reps and geometry: closest point, point
//! classification, bounding boxes, adjacency lookups, manifold checks.
//!
//! Upstream: `SmPointClass`, `SmRayTracer`, `SmExtent*`, `SmPseudoBox`,
//! `SmTopologyTraverser` in `source/SMLib`, and `SM_API/SmApiQueries`.

pub mod api_queries;
pub mod brep;
pub mod contact;
pub mod continuity;
pub mod distance_bound;
pub mod edge;
pub mod edge_turn;
#[cfg(test)]
pub(crate) mod exact;
pub mod face;
mod face_surface;
pub mod geometry_snap;
pub mod r#loop;
pub mod normal;
pub mod poly;
pub mod poly_properties;
pub mod scan_remesh;
pub mod scan_surface_fit;
pub mod scan_symmetry;
pub mod scan_surface_patch;
pub mod tangent_edge;
pub mod tess;

pub use face_surface::face_surface_intersect;

/// Exact matching of trimmed fitted patches.
pub mod surface_match;
