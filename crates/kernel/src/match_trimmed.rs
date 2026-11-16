//! Explicit-edge matching of fitted trimmed patches through solidcore operations.
use crate::history::{Change, EdgeProv, History};
use crate::{Body, Continuity, KernelError, Result};
pub use solidcore_ops::surface_match::MatchReport;
use solidcore_record::Vec3;
/// Match one explicitly picked source edge to one fixed neighbour edge.
pub fn matched_trimmed(
    source: &Body,
    source_edges: &[Vec3],
    target: &Body,
    target_edges: &[Vec3],
    order: Continuity,
    cross_scale: f64,
    reverse_target: bool,
) -> Result<(Body, MatchReport)> {
    source.require_brep("match fitted patch")?;
    target.require_brep("match fitted patch")?;
    if !source.is_surface() || !target.is_surface() {
        return Err(KernelError::Invalid("trimmed patch matching requires two surface bodies".into()));
    }
    let pick = |body: &Body, points: &[Vec3]| -> Result<usize> {
        let [point] = points else {
            return Err(KernelError::Invalid("pick exactly one source edge and one target edge for trimmed matching".into()));
        };
        crate::surfaces::edge_near(body, *point).ok_or_else(|| KernelError::Invalid("selected point is not on a patch boundary edge".into()))
    };
    let source_edge = pick(source, source_edges)?;
    let target_edge = pick(target, target_edges)?;
    use solidcore_ops::surface_match::{MatchOptions, MatchOrder};
    let order = match order {
        Continuity::G0 => MatchOrder::G0,
        Continuity::G1 => MatchOrder::G1,
        Continuity::G2 => MatchOrder::G2,
        _ => return Err(KernelError::Invalid("exact trimmed patch matching supports G0, G1 and G2".into())),
    };
    let (record, report) = solidcore_ops::surface_match::match_trimmed(
        source.record(),
        source_edge,
        target.record(),
        target_edge,
        MatchOptions { order, cross_scale, reverse_target },
    )
    .map_err(|e| KernelError::Failed(e.to_string()))?;
    let body = Body::from_record(record)?;
    let history = History {
        operands: 2,
        faces: vec![crate::record::from_face(0, 0, Change::Modified)],
        edges: (0..body.edge_count()).map(|_| EdgeProv { faces: vec![0], origins: Vec::new() }).collect(),
    };
    Ok((body.with_history(history, &[source, target])?, report))
}
