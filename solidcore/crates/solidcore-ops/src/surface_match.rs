//! The exact trimmed-patch matching recipe; all geometry belongs to the kernel.
use crate::body::{finish, Body, Expect};
use crate::session::run;
use crate::{Error, Result};
pub use solidcore::query::surface_match::{MatchOptions, MatchOrder, MatchReport};
use solidcore::query::surface_match::{PatchInput, TrimSegment};
use solidcore::SolidError;
use solidcore_record::brep::{Brep as Record, Curve2, Loop, Surface};
fn input(record: &Record) -> Result<PatchInput> {
    if record.faces.len() != 1 {
        return Err(Error::Unsupported(
            "exact trimmed matching requires a one-face fitted surface",
        ));
    }
    let face = record.faces.first().ok_or(Error::Empty)?;
    let Surface::Spline {
        u_knots,
        v_knots,
        points,
    } = &face.surface
    else {
        return Err(Error::Unsupported(
            "exact trimmed matching requires a nonrational fitted polynomial patch",
        ));
    };
    let mut loops = Vec::new();
    for ring in &face.loops {
        let Loop::Edges { edges } = ring else {
            return Err(Error::Unsupported(
                "collapsed vertex trim loops are not supported by fitted-patch matching",
            ));
        };
        let mut out = Vec::new();
        for edge in edges {
            let Curve2::Spline { knots, points } = &edge.pcurve else {
                return Err(Error::Unsupported(
                    "matching requires straight polynomial UV trim segments",
                ));
            };
            if points.len() > 128 || knots.len() > 141 {
                return Err(Error::Unsupported(
                    "trim pcurve exceeds the fitted-patch matching budget",
                ));
            }
            let original = record
                .edges
                .get(edge.edge as usize)
                .ok_or(Error::Argument)?;
            out.push(TrimSegment {
                edge_index: edge.edge as usize,
                points: points.clone(),
                knots: knots.clone(),
                interval: original.interval,
                forward: edge.forward,
            });
        }
        loops.push(out);
    }
    Ok(PatchInput {
        points: points.clone(),
        u_knots: u_knots.clone(),
        v_knots: v_knots.clone(),
        loops,
    })
}
/// Move the source patch only; both input records remain unchanged.
pub fn match_trimmed(
    source: &Record,
    source_edge: usize,
    target: &Record,
    target_edge: usize,
    options: MatchOptions,
) -> Result<(Record, MatchReport)> {
    let source = input(source)?;
    let target = input(target)?;
    let (body, report) = run("surface_match::match_trimmed", |k| {
        let (body, report) = match solidcore::query::surface_match::match_trimmed(
            k,
            &source,
            source_edge,
            &target,
            target_edge,
            options,
        ) {
            Ok(v) => v,
            Err(SolidError::InvalidInput { reason, .. }) => {
                return Ok(Err(Error::Unsupported(reason)))
            }
            Err(e) => return Err(e),
        };
        Ok(finish(k, body, Expect::FACES)?.map(|body| (body, report)))
    })??;
    let (record, _) = Body::new(body).to_record()?;
    Ok((record, report))
}
