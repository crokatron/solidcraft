//! Editable triangle remeshing through the guarded numerical kernel.
pub use solidcore::query::scan_remesh::{Attributes, Goal, Report, Scan};
impl crate::session::Answer for (Scan, Report) {}
/// Preserve boundaries and appearance charts while reducing or redistributing triangles.
pub fn run(scan: Scan, goal: Goal) -> crate::Result<(Scan, Report)> {
    crate::session::run("scan_remesh::run", |_| {
        solidcore::query::scan_remesh::run(scan, goal)
    })
}
