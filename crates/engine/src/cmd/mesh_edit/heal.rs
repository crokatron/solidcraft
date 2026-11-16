use super::{Body, EditResult, edit, parse};
use crate::{Result, Session, params::bad};
use schemars::JsonSchema;
use serde_json::{Value, json};

#[derive(JsonSchema, serde::Serialize)]
pub(super) struct HealResult {
    #[serde(flatten)]
    edit: EditResult,
    removed_facets: usize,
    removed_vertices: usize,
    reoriented_facets: usize,
    added_facets: usize,
    skipped: Vec<String>,
    heal_report: String,
}

pub(super) fn run(session: &mut Session, params: &Value) -> Result<Value> {
    const CMD: &str = "mesh.heal_scan";
    let args: Body = parse(params, CMD)?;
    let mut removed_facets = 0;
    let mut removed_vertices = 0;
    let mut reoriented = 0;
    let mut added = 0;
    let mut skipped = Vec::new();
    let mut result = edit(session, CMD, args.body.as_deref(), |mesh, _| {
        let old_faces = mesh.triangles.len() / 3;
        let old_vertices = mesh.positions.len() / 3;
        mesh.clean_scan().map_err(|e| bad(CMD, e.to_string()))?;
        let clean_faces = mesh.triangles.len() / 3;
        removed_facets = old_faces - clean_faces;
        removed_vertices = old_vertices - mesh.positions.len() / 3;
        let topology = mesh.scan_topology().map_err(|e| bad(CMD, e.to_string()))?;
        if topology.nonmanifold_edges > 0 {
            skipped.push(format!(
                "Orientation and hole filling skipped because {} edges have more than two incident triangles.",
                topology.nonmanifold_edges
            ));
            return Ok(());
        }
        if topology.winding_conflicts > 0 {
            let original = mesh.triangles.clone();
            if let Err(e) = mesh.orient_scan_normals() {
                skipped.push(format!("Orientation and hole filling skipped: {e}."));
                return Ok(());
            }
            reoriented = original.chunks_exact(3).zip(mesh.triangles.chunks_exact(3)).filter(|(a, b)| a != b).count();
        }
        if topology.boundary_edges > 0 {
            match mesh.fill_scan_holes(32) {
                Ok(()) => added = mesh.triangles.len() / 3 - clean_faces,
                Err(e) => skipped.push(format!("Hole filling skipped: {e}.")),
            }
        }
        Ok(())
    })?;
    let mut report = format!(
        "Removed {removed_facets} duplicate or degenerate triangles and {removed_vertices} redundant vertices. Reoriented {reoriented} triangles and added {added} patch triangles."
    );
    for message in &skipped {
        report.push(' ');
        report.push_str(message);
    }
    result["removed_facets"] = json!(removed_facets);
    result["removed_vertices"] = json!(removed_vertices);
    result["reoriented_facets"] = json!(reoriented);
    result["added_facets"] = json!(added);
    result["skipped"] = json!(skipped);
    result["heal_report"] = json!(report);
    Ok(result)
}
