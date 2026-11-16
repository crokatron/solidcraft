//! Bounded scan surfaces: selection and persistent fit recipes; geometry lives in solidcore.
use super::*;
use serde::Serialize;
use solidcraft_kernel::scan_surface_fit::{self as fit, Kind, Options};
use std::collections::BTreeMap;

const CMD: &str = "mesh.fit_surface";
const MAX_FACETS: usize = 1_000_000;

#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct Args {
    pub body: Option<String>,
    pub triangle: Option<usize>,
    pub point: Option<[f64; 3]>,
    pub auto_region: Option<u32>,
    pub angle: Option<f64>,
    /// Inclusive, sorted source facet ranges, from a lasso selection.
    pub ranges: Option<Vec<[u32; 2]>>,
    pub revision: Option<u64>,
    /// plane, cylinder, cone, or freeform (one bicubic height patch).
    pub kind: String,
    /// Target sampled error, in millimetres; reported, not a continuous error certificate.
    pub tolerance: Option<f64>,
    pub boundary_tolerance: Option<f64>,
    /// Preferred surface U direction in world coordinates.
    pub flow: Option<[f64; 3]>,
    pub fairing: Option<f64>,
    pub name: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub(super) struct FitResult {
    feature: u64,
    body: String,
    region_triangles: usize,
    rms_mm: f64,
    max_mm: f64,
    within_tolerance: bool,
    boundary_loops: usize,
    boundary_vertices: usize,
    control_points_u: usize,
    control_points_v: usize,
    fit_report: String,
    metric: String,
    sampled_points: usize,
    parameters: BTreeMap<String, f64>,
}

fn members(args: &Args, src: &Source<'_>) -> Result<Vec<u32>> {
    if let Some(ranges) = &args.ranges {
        if args.point.is_some() || args.triangle.is_some() || args.auto_region.is_some() {
            return Err(bad(CMD, "Choose a lasso patch or a connected region, not both."));
        }
        let mut result = Vec::new();
        let mut previous = None;
        for &[lo, hi] in ranges {
            if lo > hi || hi as usize >= src.mesh.triangles.len() / 3 || previous.is_some_and(|p| lo <= p) {
                return Err(bad(CMD, "Facet ranges are invalid or overlap. Select the patch again."));
            }
            let count = (hi as usize).saturating_sub(lo as usize).saturating_add(1);
            if result.len().saturating_add(count) > MAX_FACETS {
                return Err(bad(CMD, "Choose a smaller patch (at most one million facets)."));
            }
            result.extend(lo..=hi);
            previous = Some(hi);
        }
        if result.is_empty() {
            return Err(bad(CMD, "Select a scan patch first."));
        }
        return Ok(result);
    }
    let region: region::Region = parse(
        &json!({"body":args.body,"triangle":args.triangle,"point":args.point,
        "auto_region":args.auto_region,"angle":args.angle}),
        CMD,
    )?;
    let result = region::members(&region, src, CMD, 20.)?;
    if result.len() > MAX_FACETS {
        return Err(bad(CMD, "Choose a smaller connected region (at most one million facets)."));
    }
    Ok(result)
}

/// Index compaction and coordinate-frame conversion only; no geometric decisions here.
fn selected_data(src: &Source<'_>, indices: &[u32]) -> Result<(Vec<[f64; 3]>, Vec<[u32; 3]>, String)> {
    let mut vertices = BTreeMap::new();
    let mut positions = Vec::new();
    let mut triangles = Vec::new();
    let source_positions = src.mesh.positions.as_chunks::<3>().0;
    let source_faces = src.mesh.triangles.as_chunks::<3>().0;
    let mut hash = 14695981039346656037u64;
    let mut hash_word = |word: u64| {
        for b in word.to_le_bytes() {
            hash = (hash ^ u64::from(b)).wrapping_mul(1099511628211);
        }
    };
    hash_word(source_faces.len() as u64);
    hash_word(source_positions.len() as u64);
    for &index in indices {
        hash_word(index as u64);
        let face = source_faces.get(index as usize).ok_or_else(|| bad(CMD, "Selected facet no longer exists."))?;
        let mut mapped = [0; 3];
        for (slot, &vertex) in mapped.iter_mut().zip(face) {
            hash_word(vertex as u64);
            let local = source_positions.get(vertex as usize).ok_or_else(|| bad(CMD, "Invalid scan vertex index."))?;
            for &value in local {
                hash_word(value.to_bits());
            }
            *slot = if let Some(&n) = vertices.get(&vertex) {
                n
            } else {
                let n = u32::try_from(positions.len()).map_err(|_| bad(CMD, "Patch vertex budget exceeded."))?;
                let world = apply_point(&src.frame, Vec3::new(local[0], local[1], local[2]));
                positions.push([world.x, world.y, world.z]);
                vertices.insert(vertex, n);
                n
            };
        }
        triangles.push(mapped);
    }
    Ok((positions, triangles, format!("{hash:016x}")))
}

fn options(args: &Args) -> Result<Options> {
    let kind = match args.kind.as_str() {
        "plane" => Kind::Plane,
        "cylinder" => Kind::Cylinder,
        "cone" => Kind::Cone,
        "freeform" => Kind::Freeform,
        _ => return Err(bad(CMD, "Surface kind must be plane, cylinder, cone or freeform.")),
    };
    Ok(Options {
        kind,
        tolerance: args.tolerance.unwrap_or(0.5),
        boundary_tolerance: args.boundary_tolerance.unwrap_or(0.5),
        flow: args.flow,
        fairing: args.fairing.unwrap_or(0.0001),
    })
}

pub(super) fn run(s: &mut Session, p: &Value) -> Result<Value> {
    run_into(s, parse(p, CMD)?, None)
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct Refit {
    feature: u64,
    auto_flow: Option<bool>,
    kind: Option<String>,
    tolerance: Option<f64>,
    boundary_tolerance: Option<f64>,
    flow: Option<[f64; 3]>,
    fairing: Option<f64>,
}

pub(super) fn refit(s: &mut Session, p: &Value) -> Result<Value> {
    let change: Refit = parse(p, "mesh.fit_surface.refit")?;
    let saved = s
        .doc
        .metadata
        .0
        .get(&format!("scan_surface_fit:{}", change.feature))
        .ok_or_else(|| bad(CMD, "This feature has no saved scan fitting recipe."))?
        .clone();
    let mut args: Args = parse(&saved["options"], CMD)?;
    let feature = s.doc.feature(change.feature).ok_or_else(|| bad(CMD, "Fitted surface no longer exists."))?;
    if feature.component != s.active_component {
        return Err(bad(CMD, "Activate the component containing the fitted surface before refitting."));
    }
    if !matches!(feature.kind, FeatureKind::Import { .. }) {
        return Err(bad(CMD, "The fitted base surface has changed type."));
    }
    let src = source(s, CMD, args.body.as_deref())?;
    if Some(src.feature) != saved["source_feature"].as_u64() || Some(src.index as u64) != saved["source_index"].as_u64() {
        return Err(bad(CMD, "The source scan changed. Select the patch again."));
    }
    let indices = members(&args, &src)?;
    let (_, _, hash) = selected_data(&src, &indices)?;
    if Some(hash.as_str()) != saved["fingerprint"].as_str() {
        return Err(bad(CMD, "The saved facet selection is stale after scan edits. Select the patch again."));
    }
    if let Some(kind) = change.kind {
        args.kind = kind;
    }
    if let Some(value) = change.tolerance {
        args.tolerance = Some(value);
    }
    if let Some(value) = change.boundary_tolerance {
        args.boundary_tolerance = Some(value);
    }
    if change.auto_flow == Some(true) && change.flow.is_none() {
        args.flow = None;
    }
    if let Some(value) = change.flow {
        args.flow = Some(value);
    }
    if let Some(value) = change.fairing {
        args.fairing = Some(value);
    }
    run_into(s, args, Some(change.feature))
}

fn run_into(s: &mut Session, args: Args, replace: Option<u64>) -> Result<Value> {
    if args.revision.is_some_and(|r| r != s.revision) {
        return Err(bad(CMD, "The design changed; select the scan patch again."));
    }
    let src = source(s, CMD, args.body.as_deref())?;
    let indices = members(&args, &src)?;
    let (positions, triangles, fingerprint) = selected_data(&src, &indices)?;
    let source_feature = src.feature;
    let source_index = src.index;
    let source_name = src.name.clone();
    let resolved = options(&args)?;
    let (geometry, mut report) = fit::fit(&positions, &triangles, &resolved).map_err(|e| bad(CMD, e.to_string()))?;
    if report.metric == "frame_normal_sampled" {
        report.metric = "Distance along the fitted frame normal, measured at vertices and facet centres; excludes trim-boundary distance.".into();
    }
    let inv = mat_inverse(&s.active_component_frame()).ok_or_else(|| bad(CMD, "Active component placement is singular."))?;
    let local = solidcraft_kernel::transform_matrix(&geometry, inv)?;
    let step = solidcraft_kernel::step_export(&[&local], "SolidCraft bounded scan surface")?;
    let kind = FeatureKind::Import { file: format!("Fitted {} surface", args.kind), step: step.into(), components: Vec::new(), skip: Vec::new(), meshes: Vec::new() };
    let feature = if let Some(feature) = replace {
        s.doc_mut().feature_mut(feature).ok_or_else(|| bad(CMD, "Fitted surface is missing."))?.kind = kind;
        s.refresh();
        if let Some(error) = s.model.result(feature).and_then(|r| r.error.as_ref()) {
            return Err(bad(CMD, error));
        }
        feature
    } else {
        let result = super::super::features::add_feature_in(s, &json!({"name":args.name.as_deref().unwrap_or("Fitted scan surface")}), kind, false)?;
        result["feature"].as_u64().ok_or_else(|| bad(CMD, "Fitted feature is missing."))?
    };
    let body = s
        .model
        .state()
        .bodies
        .iter()
        .find(|b| b.feature == feature)
        .map(|b| b.name.clone())
        .ok_or_else(|| bad(CMD, "Fitted surface did not evaluate."))?;
    let mut ranges = Vec::new();
    for index in &indices {
        facet_mask::append(&mut ranges, *index);
    }
    let mut recipe = args.clone();
    recipe.tolerance = Some(resolved.tolerance);
    recipe.boundary_tolerance = Some(resolved.boundary_tolerance);
    recipe.fairing = Some(resolved.fairing);
    recipe.body = Some(source_name);
    recipe.ranges = Some(ranges);
    recipe.point = None;
    recipe.triangle = None;
    recipe.auto_region = None;
    recipe.revision = None;
    s.doc_mut().metadata.0.insert(
        format!("scan_surface_fit:{feature}"),
        json!({"version":1,"source_feature":source_feature,
        "source_index":source_index,"fingerprint":fingerprint,"options":recipe}),
    );
    // Report assembly follows the kernel's sampled metric, never a Hausdorff claim.
    let continuity = if args.kind == "freeform" { " Adjacent-patch tangency and curvature are not constrained by this fit." } else { "" };
    let fit_report = format!("Created a bounded {} CAD sheet from {} facets.{}", args.kind, indices.len(), continuity);
    Ok(json!(FitResult {
        feature,
        body,
        region_triangles: indices.len(),
        rms_mm: report.rms_mm,
        max_mm: report.max_mm,
        within_tolerance: report.within_tolerance,
        boundary_loops: report.boundary_loops,
        boundary_vertices: report.boundary_vertices,
        control_points_u: report.control_points_u,
        control_points_v: report.control_points_v,
        fit_report,
        metric: report.metric,
        sampled_points: report.sampled_points,
        parameters: report.parameters
    }))
}
