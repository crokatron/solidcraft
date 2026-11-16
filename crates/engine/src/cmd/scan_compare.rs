//! Directed, unsigned scan-to-scan distances. Every sampled point is compared with
//! the full triangle surface of the reference, independently of its tessellation density.
use super::CommandSpec;
use crate::{
    Result, Session,
    params::bad,
    schema::{TypedContract, output_schema_for},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use solidcore_record::Vec3;
use solidcraft_geom::{Mesh, compare::TriangleTree, prelude::*};
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Params {
    body: String,
    reference: String,
    samples: Option<usize>,
    tolerance: Option<f64>,
}
#[derive(Serialize, JsonSchema)]
struct Overlay {
    point: [f64; 3],
    distance_mm: f64,
}
#[derive(Serialize, JsonSchema)]
struct Report {
    body: String,
    reference: String,
    samples: usize,
    tolerance_mm: f64,
    mean_mm: f64,
    rms_mm: f64,
    max_mm: f64,
    coverage_percent: f64,
    overlay: Vec<Overlay>,
}
const ID: &str = "mesh.compare_scans";
pub(super) static COMMANDS:&[CommandSpec]=&[
    CommandSpec::new(ID,"Compare Scans",crate::checked!("mesh.compare_scans",compare_scans)).icon("inspect").noundo().contract(&TypedContract::<Params>::new()).output(output_schema_for::<Report>).params("Unsigned distances from area-stratified samples on body to the full reference triangle surface in world coordinates. tolerance in mm (default 1), samples 1 to 65536 (default 4096). coverage_percent is the fraction within tolerance. Overlay includes up to 512 sampled world points and distances."),
];

fn tri_area(t: [Vec3; 3]) -> f64 {
    (t[1] - t[0]).cross(t[2] - t[0]).len() * 0.5
}
fn surface_samples(mesh: &Mesh, count: usize) -> Vec<Vec3> {
    let triangles = || mesh.triangles.iter().filter_map(|f| mesh.tri(f));
    let total: f64 = triangles().map(tri_area).sum();
    if !total.is_finite() || total <= 0. {
        return vec![];
    }
    let mut out = Vec::with_capacity(count);
    let mut cumulative = 0.;
    for t in triangles() {
        let area = tri_area(t);
        if area <= 0. {
            continue;
        }
        cumulative += area;
        while out.len() < count && (out.len() as f64 + 0.5) * total / (count as f64) <= cumulative {
            // Low-discrepancy barycentric positions distribute samples inside large facets.
            let i = (out.len() + 1) as f64;
            let r = (i * 0.7548776662466927).fract().sqrt();
            let v = (i * 0.5698402909980532).fract();
            out.push(t[0] * (1. - r) + t[1] * (r * (1. - v)) + t[2] * (r * v));
        }
    }
    out
}
fn compare_scans(s: &mut Session, p: &Value) -> Result<Value> {
    let a: Params = serde_json::from_value(p.clone()).map_err(|e| bad(ID, e.to_string()))?;
    let count = a.samples.unwrap_or(4096);
    let tolerance = a.tolerance.unwrap_or(1.0);
    if !(1..=65536).contains(&count) {
        return Err(bad(ID, "samples must be from 1 through 65536"));
    }
    if !tolerance.is_finite() || tolerance < 0. {
        return Err(bad(ID, "tolerance must be a finite nonnegative distance"));
    }
    if a.body == a.reference {
        return Err(bad(ID, "select two different scans"));
    }
    let state = s.world_state()?;
    let get = |n: &str| state.body(n).filter(|b| b.body.is_mesh()).map(|b| b.mesh()).ok_or_else(|| bad(ID, format!("select an imported scan: {n}")));
    let source = get(&a.body)?;
    let reference = get(&a.reference)?;
    let points = surface_samples(&source, count);
    if points.len() != count {
        return Err(bad(ID, "source scan has no measurable surface area"));
    }
    let tree = TriangleTree::new(&reference);
    if tree.is_empty() {
        return Err(bad(ID, "reference scan has no triangles"));
    }
    let distances = solidcraft_kernel::map_on_threads(&points, 16, |p| tree.nearest(*p).map(|(d, _)| d));
    let mut sum = 0.;
    let mut squares = 0.;
    let mut max = 0.0_f64;
    let mut covered = 0;
    let mut overlay = Vec::new();
    let cap = count.min(512);
    for (i, dist) in distances.into_iter().enumerate() {
        let d = dist.flatten().ok_or_else(|| bad(ID, "could not measure a sample on the reference scan"))?;
        sum += d;
        squares += d * d;
        max = max.max(d);
        if d <= tolerance {
            covered += 1;
        }
        if (i + 1) * cap / count != i * cap / count {
            let p = points[i];
            overlay.push(Overlay { point: [p.x, p.y, p.z], distance_mm: d });
        }
    }
    Ok(json!(Report {
        body: a.body,
        reference: a.reference,
        samples: count,
        tolerance_mm: tolerance,
        mean_mm: sum / count as f64,
        rms_mm: (squares / count as f64).sqrt(),
        max_mm: max,
        coverage_percent: 100. * covered as f64 / count as f64,
        overlay
    }))
}

#[cfg(test)]
mod tests {
    use crate::{
        Session,
        doc::{Document, FeatureKind, MeshData},
    };
    use serde_json::json;
    #[test]
    fn unsigned_scan_deviation_uses_triangle_interiors_with_world_units() {
        let mut doc = Document::default();
        let patch = |name: &str, z: f64| MeshData {
            appearance: None,
            name: name.into(),
            positions: vec![0., 0., z, 12., 0., z, 0., 8., z].into(),
            triangles: vec![0, 1, 2].into(),
            color: None,
            corner_normals: Default::default(),
        };
        doc.add_feature(FeatureKind::MeshImport { file: "pair.obj".into(), meshes: vec![patch("Source", 3.), patch("Reference", 0.)] }, None)
            .unwrap();
        doc.features[0].body_names = vec!["Source".into(), "Reference".into()];
        let mut s = Session::new(doc);
        let original = s.doc.to_json();
        for (tol, expected) in [(2.5, 0.), (3.5, 100.)] {
            let r = s.execute("mesh.compare_scans", &json!({"body":"Source","reference":"Reference","samples":768,"tolerance":tol})).unwrap();
            for k in ["mean_mm", "rms_mm", "max_mm"] {
                assert!((r[k].as_f64().unwrap() - 3.).abs() < 1e-9);
            }
            assert_eq!(r["samples"], 768);
            assert_eq!(r["overlay"].as_array().unwrap().len(), 512);
            assert_eq!(r["coverage_percent"].as_f64().unwrap(), expected);
        }
        assert_eq!(original, s.doc.to_json());
    }
}
