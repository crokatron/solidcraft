//! Direct edits of the original triangle data of an imported body.
use super::CommandSpec;
use crate::{
    Result, Sel, Session,
    params::bad,
    schema::{TypedContract, output_schema_for},
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use solidcore_record::Vec3;
use solidcraft_doc::{FeatureKind, MeshData, apply_point, mat_inverse};

mod alignment;
mod cloud_copy;
pub(super) use cloud_copy::shown as cloud_sample_shown;
mod auto_regions;
mod cloud_outline;
mod facet_mask;
mod facets;
mod region;
mod remesh;
use region::{Region, RegionResult, delete_region, isolate_region, select_region};
mod fit_plane;
mod heal;
pub(super) use fit_plane::principal_axes;
mod fit_cylinder;
mod fit_surface;
mod symmetry;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Pick {
    body: Option<String>,
    /// World-space point from a viewport pick, or an explicit source triangle index.
    point: Option<[f64; 3]>,
    triangle: Option<usize>,
    /// For an explicit triangle, its local vertex or edge (0, 1, or 2).
    corner: Option<usize>,
}
#[derive(Deserialize, JsonSchema)]
struct Move {
    #[serde(flatten)]
    pick: Pick,
    /// Displacement in the source mesh's local axes, in millimetres.
    delta: [f64; 3],
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Body {
    body: Option<String>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Smooth {
    body: Option<String>,
    iterations: Option<u32>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Decimate {
    body: Option<String>,
    keep_percent: Option<u32>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct FillHoles {
    body: Option<String>,
    /// Maximal number of edges around a planar boundary to close, from 3 through 256.
    max_edges: Option<u32>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Islands {
    body: Option<String>,
    min_triangles: Option<u32>,
}
#[derive(JsonSchema, serde::Serialize)]
pub(super) struct EditResult {
    body: String,
    feature: u64,
    triangles_before: usize,
    triangles_after: usize,
    vertices_after: usize,
}
#[derive(JsonSchema, serde::Serialize)]
struct DecimateResult {
    #[serde(flatten)]
    edit: EditResult,
    requested_triangles: usize,
    achieved_triangles: usize,
    achieved_keep_percent: f64,
    target_limited: bool,
    appearance_pinned_vertices: usize,
    max_vertex_motion_bound_mm: f64,
    decimate_report: String,
}
#[derive(JsonSchema, serde::Serialize)]
struct TopologyReport {
    body: String,
    triangles: usize,
    boundary_edges: usize,
    nonmanifold_edges: usize,
    winding_conflicts: usize,
    degenerate_triangles: usize,
}

fn inspect_topology(s: &mut Session, p: &Value) -> Result<Value> {
    let id = "mesh.inspect_topology";
    let args: Body = parse(p, id)?;
    let source = source(s, id, args.body.as_deref())?;
    let report = source.mesh.scan_topology().map_err(|e| bad(id, e.to_string()))?;
    Ok(json!(TopologyReport {
        body: source.name,
        triangles: report.triangles,
        boundary_edges: report.boundary_edges,
        nonmanifold_edges: report.nonmanifold_edges,
        winding_conflicts: report.winding_conflicts,
        degenerate_triangles: report.degenerate_triangles
    }))
}
pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("mesh.symmetry_plane", "Define Scan Mirror Plane", crate::checked!("mesh.symmetry_plane", symmetry::run))
        .icon("mirror").contract(&TypedContract::<symmetry::Args>::new()).output(output_schema_for::<symmetry::Output>)
        .params("Define a named mirror datum linked to a scan body. Supply explicit world origin and normal, or at least three corresponding left/right world point pairs. Reports reflected-pair RMS and maximum error; no automatic whole-scan symmetry claim."),
    CommandSpec::new("mesh.align_plane_to_csys","Align Scan Plane to CSYS",crate::checked!("mesh.align_plane_to_csys",alignment::plane::run))
        .icon("align").contract(&TypedContract::<alignment::plane::Args>::new()).output(output_schema_for::<alignment::plane::Output>)
        .params("Place the selected fitted scan plane onto XY (+Z), XZ (+Y) or YZ (+X). Optional flip reverses the target normal. Optional angle_deg rotates about that signed normal at the aligned plane origin. Optional offset: [u,v] in mm translates after rotation along world X/Y for XY, X/Z for XZ, or Y/Z for YZ, independent of flip."),
    CommandSpec::new("mesh.align_landmarks","Align by Three Points",crate::checked!("mesh.align_landmarks",alignment::landmarks))
        .icon("align").contract(&TypedContract::<alignment::Landmarks>::new()).output(output_schema_for::<alignment::ResultData>)
        .params("Align two imported scans using three corresponding noncollinear world points on each. Supply moving, fixed, moving_points and fixed_points; reports landmark RMS in mm."),
    CommandSpec::new("mesh.align_datum","Align Scan to Datum",crate::checked!("mesh.align_datum",alignment::datum))
        .icon("align").contract(&TypedContract::<alignment::Datum>::new()).output(output_schema_for::<alignment::ResultData>)
        .params("Place a scan in the global coordinate system given moving, origin, x_point on the positive X axis and xy_point on the positive Y side of the XY plane, all in world mm."),
    CommandSpec::new("pointcloud.section_profile","Fit Cloud Outline",crate::checked!("pointcloud.section_profile",cloud_outline::run))
        .icon("sketch").contract(&TypedContract::<cloud_outline::Args>::new()).output(output_schema_for::<cloud_outline::Output>)
        .params("Build editable planar outlines around the projected occupied cells of visible cloud points within a world slab. Supply cloud, origin, normal, thickness and resolution in mm. Holes and gaps larger than the sample grid remain separate."),
    CommandSpec::new("mesh.copy_point_cloud","Copy to Point Cloud",crate::checked!("mesh.copy_point_cloud",cloud_copy::all))
        .icon("point_cloud").contract(&TypedContract::<cloud_copy::Whole>::new()).output(output_schema_for::<cloud_copy::Copied>)
        .params("Create an independent embedded point cloud from the distinct vertices used by an imported scan body. Optional name."),
    CommandSpec::new("mesh.copy_facet_cloud","Copy Lasso Patch",crate::checked!("mesh.copy_facet_cloud",cloud_copy::facets))
        .icon("point_cloud").contract(&TypedContract::<cloud_copy::Patch>::new()).output(output_schema_for::<cloud_copy::Copied>)
        .params("Copy the unique vertices of a source facet mask to an independent embedded point cloud. Supply body, ordered inclusive index ranges, revision, and optional name."),
    CommandSpec::new("pointcloud.copy_snippet","Copy Point Snippet",crate::checked!("pointcloud.copy_snippet",cloud_copy::snippet))
        .icon("point_cloud").contract(&TypedContract::<cloud_copy::Snippet>::new()).output(output_schema_for::<cloud_copy::Copied>)
        .params("Copy selected visible points and their properties to an independent embedded cloud. Supply cloud id or name, ordered inclusive point ranges, revision, and optional name."),
    CommandSpec::new("mesh.delete_facets","Lasso Delete",crate::checked!("mesh.delete_facets",facet_mask::delete))
        .icon("delete").contract(&TypedContract::<facet_mask::Args>::new()).output(output_schema_for::<EditResult>)
        .params("Delete selected source triangles from a scan. Supply ordered inclusive index ranges and the original session revision."),
    CommandSpec::new("mesh.isolate_facets","Lasso Isolate",crate::checked!("mesh.isolate_facets",facet_mask::isolate))
        .icon("selection").contract(&TypedContract::<facet_mask::Args>::new()).output(output_schema_for::<EditResult>)
        .params("Keep only selected source triangles from a scan, with ordered inclusive ranges and the original session revision."),
    CommandSpec::new("mesh.auto_regions", "Detect Regions", crate::checked!("mesh.auto_regions", auto_regions::run))
        .icon("selection").noundo().contract(&TypedContract::<auto_regions::Args>::new()).output(output_schema_for::<auto_regions::Output>)
        .params("Detect connected regions across the entire imported scan. Set angle in degrees to control the normal-angle threshold. Region results refer to the original triangles."),
    CommandSpec::new("mesh.global_remesh", "Global Remesh", crate::checked!("mesh.global_remesh", remesh::run))
        .icon("mesh_export").contract(&TypedContract::<remesh::Args>::new()).output(output_schema_for::<remesh::Output>)
        .params("Redistribute triangles across the entire imported scan by refining and coarsening edges. Set target_length in millimetres or omit it for automatic sizing."),
    CommandSpec::new("mesh.align_coarse","Auto Align",crate::checked!("mesh.align_coarse",alignment::coarse::run))
        .icon("align").contract(&TypedContract::<alignment::coarse::Args>::new()).output(output_schema_for::<alignment::Report>)
        .params("Rigidly place a moving scan on a fixed reference by testing 24 principal-axis orientations and refining the best sampled overlap."),
    CommandSpec::new("mesh.align_scans","Fine Align Scans",crate::checked!("mesh.align_scans",alignment::run))
        .icon("align").contract(&TypedContract::<alignment::Params>::new()).output(output_schema_for::<alignment::Report>)
        .params("Fine rigid ICP alignment of two overlapping, roughly aligned imported scans. moving and fixed name the source and reference bodies. iterations defaults to 30, range 1–100. Report trimmed sampled nearest-vertex RMS in mm."),
    CommandSpec::new("mesh.select_region","Select Mesh Region",crate::checked!("mesh.select_region",select_region))
        .icon("selection").noundo().contract(&TypedContract::<Region>::new()).output(output_schema_for::<RegionResult>)
        .params("Return source triangle indices connected across shared edges whose normals lie within angle degrees of the seed facet (default 20)."),
    CommandSpec::new("mesh.delete_region","Delete Mesh Region",crate::checked!("mesh.delete_region",delete_region))
        .icon("delete").contract(&TypedContract::<Region>::new()).output(output_schema_for::<EditResult>)
        .params("Remove the edge-connected facets within angle degrees of the picked seed normal, preserving the rest of the scan."),
    CommandSpec::new("mesh.isolate_region","Isolate Mesh Region",crate::checked!("mesh.isolate_region",isolate_region))
        .icon("split_face").contract(&TypedContract::<Region>::new()).output(output_schema_for::<EditResult>)
        .params("Keep the edge-connected facets within angle degrees of the picked seed normal, removing other facets from the source scan."),
    CommandSpec::new("mesh.flip_edge","Flip Mesh Edge",crate::checked!("mesh.flip_edge",flip_edge))
        .icon("split_face").contract(&TypedContract::<Pick>::new()).output(output_schema_for::<EditResult>)
        .params("Replace the shared edge of two neighbouring triangles with the other diagonal of their patch."),
    CommandSpec::new("mesh.delete_island","Delete Mesh Island",crate::checked!("mesh.delete_island",delete_island))
        .icon("delete").contract(&TypedContract::<Pick>::new()).output(output_schema_for::<EditResult>)
        .params("Remove the connected mesh island containing the picked facet."),
    CommandSpec::new("mesh.fill_holes","Fill Mesh Holes",crate::checked!("mesh.fill_holes",fill_holes))
        .icon("surface_patch").contract(&TypedContract::<FillHoles>::new()).output(output_schema_for::<EditResult>)
        .params("Cap simple planar boundary loops with at most max_edges sides (default 64, range 3 to 256)."),
    CommandSpec::new("mesh.orient_normals","Orient Mesh Normals",crate::checked!("mesh.orient_normals",orient_normals))
        .icon("surface_reverse_normal").contract(&TypedContract::<Body>::new()).output(output_schema_for::<EditResult>)
        .params("Make adjacent facets wind consistently and align their stored shading normals. Preserve the majority orientation of each component."),
    CommandSpec::new("mesh.fit_region_cylinder", "Fit Region Cylinder", crate::checked!("mesh.fit_region_cylinder", fit_cylinder::run))
        .icon("cylinder").contract(&TypedContract::<region::Region>::new()).output(output_schema_for::<fit_cylinder::CylinderResult>)
        .params("Estimate a cylinder from a connected curved patch, create an editable solid cylinder, and report radial deviation."),
    CommandSpec::new("mesh.fit_region_plane", "Fit Region Plane", crate::checked!("mesh.fit_region_plane", fit_plane::run))
        .icon("plane").contract(&TypedContract::<region::Region>::new()).output(output_schema_for::<fit_plane::PlaneResult>)
        .params("Fit an area-weighted least-squares plane to connected triangles within the normal-angle tolerance and create a construction plane."),
    CommandSpec::new("mesh.fit_surface", "Fit Scan Surface", crate::checked!("mesh.fit_surface", fit_surface::run))
        .at("MESH", "REGIONS").icon("surface_patch")
        .contract(&TypedContract::<fit_surface::Args>::new()).output(output_schema_for::<fit_surface::FitResult>)
        .params("Fit a bounded CAD sheet to a connected scan region or lasso facet ranges. kind: plane|cylinder|cone|freeform; body; point/triangle/auto_region and angle, or ranges with revision; tolerance?: sampled target mm; flow?: world U direction; fairing?:0..1; name?. Keeps source scan appearance untouched. Reports sampled error, not a continuous deviation certificate."),
    CommandSpec::new("mesh.fit_surface.refit", "Refit Scan Surface", crate::checked!("mesh.fit_surface.refit", fit_surface::refit))
        .icon("surface_patch").contract(&TypedContract::<fit_surface::Refit>::new()).output(output_schema_for::<fit_surface::FitResult>)
        .params("feature: fitted surface id; kind?,tolerance?,boundary_tolerance?,flow?,fairing?. Refit the saved selection, preserving feature identity. Refuses stale scan selections. Activate the owning component first."),
    CommandSpec::new("mesh.heal_scan", "Heal Scan", crate::checked!("mesh.heal_scan", heal::run))
        .icon("surface_patch")
        .contract(&TypedContract::<Body>::new())
        .output(output_schema_for::<heal::HealResult>)
        .params("Weld coincident vertices, remove duplicate and degenerate triangles, correct inconsistent winding and fill simple planar holes up to 32 boundary edges."),
    CommandSpec::new("mesh.inspect_topology", "Inspect Mesh", crate::checked!("mesh.inspect_topology", inspect_topology))
        .icon("inspect").noundo().contract(&TypedContract::<Body>::new()).output(output_schema_for::<TopologyReport>)
        .params("Count scan triangles, boundary edges, non-manifold edges, inconsistently wound edges and degenerate triangles. Edge counts exclude degenerate triangles."),
    CommandSpec::new("mesh.move_vertex", "Move Mesh Vertex", crate::checked!("mesh.move_vertex", move_vertex))
        .icon("move")
        .contract(&TypedContract::<Move>::new())
        .output(output_schema_for::<EditResult>)
        .params("Move the vertex nearest the picked point by X, Y and Z in the mesh's local axes."),
    CommandSpec::new("mesh.split_edge", "Split Mesh Edge", crate::checked!("mesh.split_edge", split_edge))
        .icon("split_face")
        .contract(&TypedContract::<Pick>::new())
        .output(output_schema_for::<EditResult>)
        .params("Pick near a triangle edge to split every incident triangle at its shared midpoint."),
    CommandSpec::new("mesh.subdivide_face", "Subdivide Mesh Face", crate::checked!("mesh.subdivide_face", subdivide_face))
        .icon("split_body")
        .contract(&TypedContract::<Pick>::new())
        .output(output_schema_for::<EditResult>)
        .params("Divide a picked triangle into three triangles meeting at its center."),
    CommandSpec::new("mesh.delete_face", "Delete Mesh Face", crate::checked!("mesh.delete_face", delete_face))
        .icon("delete")
        .contract(&TypedContract::<Pick>::new())
        .output(output_schema_for::<EditResult>)
        .params("Remove the picked triangle and leave an opening in the mesh."),
    CommandSpec::new("mesh.cleanup", "Clean Scan", crate::checked!("mesh.cleanup", cleanup))
        .icon("surface_stitch")
        .contract(&TypedContract::<Body>::new())
        .output(output_schema_for::<EditResult>)
        .params("Weld coincident vertices and remove duplicate triangles, degenerate triangles and unused vertices."),
    CommandSpec::new("mesh.remove_islands", "Remove Small Islands", crate::checked!("mesh.remove_islands", remove_islands))
        .icon("delete")
        .contract(&TypedContract::<Islands>::new())
        .output(output_schema_for::<EditResult>)
        .params("Remove small detached groups of triangles from an imported scan; min_triangles defaults to 20. The largest group is kept."),
    CommandSpec::new("mesh.smooth", "Smooth Scan", crate::checked!("mesh.smooth", smooth))
        .icon("c_smooth")
        .contract(&TypedContract::<Smooth>::new())
        .output(output_schema_for::<EditResult>)
        .params("Smooth an imported scan while holding its boundary in place."),
    CommandSpec::new("mesh.decimate", "Decimate Scan", crate::checked!("mesh.decimate", decimate))
        .icon("mesh_export")
        .contract(&TypedContract::<Decimate>::new())
        .output(output_schema_for::<DecimateResult>)
        .params("Reduce the triangle count of a scan to a target percentage."),
];
fn parse<T: serde::de::DeserializeOwned>(p: &Value, id: &str) -> Result<T> {
    serde_json::from_value(p.clone()).map_err(|e| bad(id, e.to_string()))
}

pub(super) struct Source<'a> {
    name: String,
    feature: u64,
    index: usize,
    pub(super) mesh: &'a MeshData,
    pub(super) frame: solidcraft_doc::Mat,
}

pub(super) fn source<'a>(s: &'a Session, id: &str, name: Option<&str>) -> Result<Source<'a>> {
    let selected = || match s.selection.as_slice() {
        [Sel::Body { name }] | [Sel::Face { body: name, .. }] => Some(name.as_str()),
        _ => None,
    };
    let name = name.or_else(selected).ok_or_else(|| bad(id, "pick one imported mesh"))?.to_owned();
    let frames = s.body_frames();
    let (_, own, frame) = frames.iter().find(|(world, ..)| world == &name).ok_or_else(|| bad(id, "unknown mesh body"))?;
    let state = s.model.state();
    let b = state.body(own).ok_or_else(|| bad(id, "unknown source body"))?;
    let feature = b.feature;
    if !b.body.is_mesh() || !s.doc.component_within(s.doc.model_body_component(b), s.active_component) {
        return Err(bad(id, "pick an imported mesh in the active component or one of its children"));
    }
    let f = s.doc.feature(feature).ok_or_else(|| bad(id, "mesh import is missing"))?;
    let FeatureKind::MeshImport { meshes, .. } = &f.kind else { return Err(bad(id, "edit the original imported mesh")) };
    let after = s.model.result(feature).ok_or_else(|| bad(id, "mesh import has not been evaluated"))?;
    let index = after
        .state
        .bodies
        .iter()
        .filter(|mb| mb.feature == feature)
        .position(|mb| mb.name == *own)
        .ok_or_else(|| bad(id, "mesh import source could not be resolved"))?;
    let mesh = meshes.get(index).ok_or_else(|| bad(id, "mesh import source index is invalid"))?;
    Ok(Source { name, feature, index, mesh, frame: *frame })
}

pub(super) fn edit(
    s: &mut Session,
    id: &str,
    name: Option<&str>,
    work: impl FnOnce(&mut MeshData, &solidcraft_doc::Mat) -> Result<()>,
) -> Result<Value> {
    let Source { name, feature, index, mesh, frame } = source(s, id, name)?;
    let mut mesh = mesh.clone();
    let old = mesh.triangles.len() / 3;
    work(&mut mesh, &frame)?;
    let result = json!(EditResult {
        body: name,
        feature,
        triangles_before: old,
        triangles_after: mesh.triangles.len() / 3,
        vertices_after: mesh.positions.len() / 3
    });
    let slot = s
        .doc_mut()
        .feature_mut(feature)
        .and_then(|f| match &mut f.kind {
            FeatureKind::MeshImport { meshes, .. } => meshes.get_mut(index),
            _ => None,
        })
        .ok_or_else(|| bad(id, "mesh source changed"))?;
    *slot = mesh;
    s.refresh();
    if let Some(error) = s.model.result(feature).and_then(|f| f.error.as_ref()) {
        return Err(bad(id, error));
    }
    Ok(result)
}
fn direct(s: &mut Session, id: &str, pick: Pick, op: facets::Op) -> Result<Value> {
    let point = pick.point.or_else(|| match s.selection.as_slice() {
        [Sel::Face { body, point, .. }] if pick.body.as_ref().is_none_or(|n| n == body) && pick.triangle.is_none() => {
            Some([point.x, point.y, point.z])
        }
        _ => None,
    });
    edit(s, id, pick.body.as_deref(), |mesh, frame| {
        let point = point
            .map(|p| -> Result<Vec3> {
                let inv = mat_inverse(frame).ok_or_else(|| bad(id, "mesh placement is singular"))?;
                Ok(apply_point(&inv, Vec3::new(p[0], p[1], p[2])))
            })
            .transpose()?;
        facets::apply(mesh, point, pick.triangle, pick.corner, op).map_err(|e| bad(id, e))
    })
}
fn move_vertex(s: &mut Session, p: &Value) -> Result<Value> {
    let id = "mesh.move_vertex";
    let args: Move = parse(p, id)?;
    direct(s, id, args.pick, facets::Op::Move(args.delta))
}
fn split_edge(s: &mut Session, p: &Value) -> Result<Value> {
    let id = "mesh.split_edge";
    direct(s, id, parse(p, id)?, facets::Op::Split)
}
fn subdivide_face(s: &mut Session, p: &Value) -> Result<Value> {
    let id = "mesh.subdivide_face";
    direct(s, id, parse(p, id)?, facets::Op::Subdivide)
}
fn delete_face(s: &mut Session, p: &Value) -> Result<Value> {
    let id = "mesh.delete_face";
    direct(s, id, parse(p, id)?, facets::Op::Delete)
}
fn flip_edge(s: &mut Session, p: &Value) -> Result<Value> {
    let id = "mesh.flip_edge";
    direct(s, id, parse(p, id)?, facets::Op::FlipEdge)
}
fn delete_island(s: &mut Session, p: &Value) -> Result<Value> {
    let id = "mesh.delete_island";
    direct(s, id, parse(p, id)?, facets::Op::DeleteIsland)
}
fn fill_holes(s: &mut Session, p: &Value) -> Result<Value> {
    let id = "mesh.fill_holes";
    let args: FillHoles = parse(p, id)?;
    edit(s, id, args.body.as_deref(), |m, _| m.fill_scan_holes(args.max_edges.unwrap_or(64)).map_err(|e| bad(id, e.to_string())))
}
fn orient_normals(s: &mut Session, p: &Value) -> Result<Value> {
    let id = "mesh.orient_normals";
    let args: Body = parse(p, id)?;
    edit(s, id, args.body.as_deref(), |m, _| m.orient_scan_normals().map_err(|e| bad(id, e.to_string())))
}
fn remove_islands(s: &mut Session, p: &Value) -> Result<Value> {
    let id = "mesh.remove_islands";
    let args: Islands = parse(p, id)?;
    edit(s, id, args.body.as_deref(), |m, _| m.remove_scan_islands(args.min_triangles.unwrap_or(20)).map_err(|e| bad(id, e.to_string())))
}
fn cleanup(s: &mut Session, p: &Value) -> Result<Value> {
    let id = "mesh.cleanup";
    let args: Body = parse(p, id)?;
    edit(s, id, args.body.as_deref(), |m, _| m.clean_scan().map_err(|e| bad(id, e.to_string())))
}
fn smooth(s: &mut Session, p: &Value) -> Result<Value> {
    let id = "mesh.smooth";
    let args: Smooth = parse(p, id)?;
    edit(s, id, args.body.as_deref(), |m, _| m.smooth_scan(args.iterations.unwrap_or(1)).map_err(|e| bad(id, e.to_string())))
}
fn decimate(s: &mut Session, p: &Value) -> Result<Value> {
    let id = "mesh.decimate";
    let args: Decimate = parse(p, id)?;
    let mut report = solidcraft_doc::scan_remesh::Report::default();
    let mut result = edit(s, id, args.body.as_deref(), |m, _| {
        report = m.decimate_scan_report(args.keep_percent.unwrap_or(50)).map_err(|e| bad(id, e.to_string()))?;
        Ok(())
    })?;
    let before = result["triangles_before"].as_u64().unwrap_or(1).max(1);
    let kept = 100. * report.achieved_triangles as f64 / before as f64;
    result["requested_triangles"] = json!(report.requested_triangles);
    result["achieved_triangles"] = json!(report.achieved_triangles);
    result["achieved_keep_percent"] = json!(kept);
    result["target_limited"] = json!(report.target_limited);
    result["appearance_pinned_vertices"] = json!(report.appearance_pinned_vertices);
    result["max_vertex_motion_bound_mm"] = json!(report.max_vertex_motion_bound);
    let note = if report.target_limited { " Protected features or the pass budget limited reduction." } else { "" };
    result["decimate_report"] = json!(format!(
        "{} -> {} triangles ({kept:.1}% retained); requested {}. Texture charts and image bytes preserved.{}",
        before, report.achieved_triangles, report.requested_triangles, note
    ));
    Ok(result)
}
