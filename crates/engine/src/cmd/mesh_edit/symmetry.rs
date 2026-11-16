//! Explicit, reproducible mirror datum definition; numerical fitting lives in solidcore.
use super::*;
use solidcraft_doc::{PlaneRef, apply_vector};
use solidcraft_geom::PlaneGeom;
use solidcraft_kernel::scan_symmetry as symmetry;

const CMD: &str = "mesh.symmetry_plane";
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct Args {
    body: String,
    origin: Option<[f64; 3]>,
    normal: Option<[f64; 3]>,
    pairs: Option<Vec<[[f64; 3]; 2]>>,
    name: Option<String>,
}
#[derive(serde::Serialize, JsonSchema)]
pub(super) struct Output {
    feature: u64,
    body: String,
    origin: [f64; 3],
    normal: [f64; 3],
    pairs: usize,
    rms_mm: Option<f64>,
    max_mm: Option<f64>,
    align_report: String,
}
pub(super) fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let args: Args = parse(p, CMD)?;
    let src = source(s, CMD, Some(&args.body))?;
    let owner = json!([src.feature, src.index]);
    let (fitted, count, rms, max) = if let Some(pairs) = &args.pairs {
        if args.origin.is_some() || args.normal.is_some() {
            return Err(bad(CMD, "Choose paired landmarks or an explicit origin and normal."));
        }
        let fit = symmetry::fit_pairs(pairs).map_err(|e| bad(CMD, e.to_string()))?;
        (fit.plane, fit.pairs, Some(fit.rms_mm), Some(fit.max_mm))
    } else {
        let origin =
            args.origin.ok_or_else(|| bad(CMD, "Provide the mirror plane origin and normal, or at least three corresponding point pairs."))?;
        let normal = args.normal.ok_or_else(|| bad(CMD, "Provide the mirror plane normal."))?;
        (symmetry::explicit_plane(origin, normal).map_err(|e| bad(CMD, e.to_string()))?, 0, None, None)
    };
    let inverse = mat_inverse(&s.active_component_frame()).ok_or_else(|| bad(CMD, "Active component placement is singular."))?;
    let point = |p: [f64; 3]| Vec3::new(p[0], p[1], p[2]);
    let plane = solidcore_record::Plane::from_normal(apply_point(&inverse, point(fitted.origin)), apply_vector(&inverse, point(fitted.normal)))
        .ok_or_else(|| bad(CMD, "Cannot construct the mirror plane."))?;
    let name = match args.name.as_deref().map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => name.to_owned(),
        None => {
            let base = "Scan mirror plane";
            (1..=s.doc.features.len() + 1)
                .map(|i| if i == 1 { base.to_owned() } else { format!("{base} ({i})") })
                .find(|name| !s.doc.features.iter().any(|feature| feature.name == *name))
                .ok_or_else(|| bad(CMD, "Could not allocate a unique mirror plane name."))?
        }
    };
    let result = super::super::features::add_feature(
        s,
        &json!({"name":name}),
        FeatureKind::ConstructionPlane { plane: PlaneRef::Custom { plane } },
    )?;
    let feature = result["feature"].as_u64().ok_or_else(|| bad(CMD, "Mirror plane feature is missing."))?;
    s.doc_mut().metadata.0.insert(alignment::plane::owner_key(feature), owner);
    s.doc_mut().metadata.0.insert(
        format!("scan_mirror_plane:{feature}"),
        json!({
            "version":1,"method":if count == 0 {"explicit"} else {"paired_landmarks"},
            "definition_world_origin":fitted.origin,"definition_world_normal":fitted.normal,
            "definition_world_pairs":args.pairs,"rms_mm":rms,"max_mm":max,
        }),
    );
    let align_report = if count == 0 {
        "Created an explicit mirror plane. Align this plane to XY, XZ or YZ, then adjust rotation and in-plane position.".into()
    } else {
        "Created a mirror plane from corresponding landmarks. Residuals measure the chosen pairs, not whole-scan symmetry. Align this plane to a coordinate-system plane, then adjust rotation and in-plane position.".into()
    };
    Ok(json!(Output { feature, body: args.body, origin: fitted.origin, normal: fitted.normal, pairs: count, rms_mm: rms, max_mm: max, align_report }))
}
