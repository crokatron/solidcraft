//! Cutting planes for imported scans, including placed occurrences.
mod automatic;
use super::{CommandSpec, mesh_edit};
use crate::{
    Result, Session,
    params::bad,
    schema::{TypedContract, output_schema_for},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use solidcore_record::{Plane, Vec3};
use solidcraft_doc::{FeatureKind, Mat, PlaneRef, apply_point};
use solidcraft_geom::{PlaneGeom, Vec3Geom};
use solidcraft_sketch::{CurveKind, Sketch};

#[derive(Default, Deserialize, JsonSchema, Clone, Copy)]
enum Principal {
    #[default]
    XY,
    XZ,
    YZ,
}
impl Principal {
    fn index(self) -> usize {
        match self {
            Self::XY => 2,
            Self::XZ => 1,
            Self::YZ => 0,
        }
    }
    fn plane(self, offset: f64) -> Plane {
        let (axis, x, y) = match self {
            Self::XY => (Vec3::Z, Vec3::X, Vec3::Y),
            Self::XZ => (Vec3::Y, Vec3::X, Vec3::Z),
            Self::YZ => (Vec3::X, Vec3::Y, Vec3::Z),
        };
        Plane { origin: axis * offset, x, y }
    }
}
/// An oriented world plane and its signed positive half-space.
fn cutting_plane(principal: Principal, direction: Option<[f64; 3]>, offset: f64, id: &str) -> Result<(Plane, Vec3)> {
    if !offset.is_finite() {
        return Err(bad(id, "enter a finite plane offset"));
    }
    let Some([x, y, z]) = direction else {
        return Ok((principal.plane(offset), [Vec3::X, Vec3::Y, Vec3::Z][principal.index()]));
    };
    let raw = Vec3::new(x, y, z);
    let n = raw
        .normalized()
        .filter(|v| v.x.is_finite() && v.y.is_finite() && v.z.is_finite() && v.len2() > 0.99)
        .ok_or_else(|| bad(id, "enter a finite nonzero plane normal"))?;
    let tangent = if n.x.abs() < 0.8 { Vec3::X } else { Vec3::Y };
    let x = (tangent - n * n.dot(tangent)).normalized().ok_or_else(|| bad(id, "invalid plane basis"))?;
    Ok((Plane { origin: n * offset, x, y: n.cross(x) }, n))
}
fn equation(frame: &Mat, normal: Vec3, offset: f64) -> [f64; 4] {
    let mut coeffs = frame.map(|v| v[0] * normal.x + v[1] * normal.y + v[2] * normal.z);
    coeffs[3] -= offset;
    coeffs
}
#[derive(Default, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Keep {
    #[default]
    Positive,
    Negative,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Trim {
    body: Option<String>,
    #[serde(default)]
    plane: Principal,
    /// Optional world direction overriding the principal plane.
    normal: Option<[f64; 3]>,
    #[serde(default)]
    offset: f64,
    #[serde(default)]
    keep: Keep,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Section {
    body: Option<String>,
    #[serde(default)]
    plane: Principal,
    /// Optional world direction overriding the principal plane.
    normal: Option<[f64; 3]>,
    #[serde(default)]
    offset: f64,
    name: Option<String>,
}
#[derive(Serialize, JsonSchema)]
struct SectionResult {
    feature: u64,
    name: String,
    curves: usize,
}
pub(super) static COMMANDS:&[CommandSpec]=&[
    CommandSpec::new("mesh.auto_sections","Auto Sections",crate::checked!("mesh.auto_sections",automatic::run)).icon("sketch").contract(&TypedContract::<automatic::Args>::new()).output(output_schema_for::<automatic::Output>).params("Create named, editable section profiles across an imported scan. Auto fits the scan direction and suggests transverse stations. Override axis, station count, offsets or maximum simplification error in mm."),
    CommandSpec::new("mesh.trim_plane","Trim Mesh",crate::checked!("mesh.trim_plane",trim)).icon("split_body").contract(&TypedContract::<Trim>::new()).output(output_schema_for::<mesh_edit::EditResult>).params("Clip the mesh at a world XY, XZ or YZ plane, or supply a world normal. For a custom direction offset is distance along the unit normal in mm. For a principal plane offset is the signed Z, Y or X coordinate in mm. keep is positive or negative. Crossing facets are split and the cut remains open."),
    CommandSpec::new("mesh.section","Mesh Section",crate::checked!("mesh.section",section)).icon("sketch").contract(&TypedContract::<Section>::new()).output(output_schema_for::<SectionResult>).params("Create an editable sketch of the mesh intersection with a world XY, XZ or YZ plane, or supply a world normal. For a custom direction offset is distance along the unit normal in mm. For a principal plane offset is the signed Z, Y or X coordinate in mm."),
];
fn trim(s: &mut Session, p: &Value) -> Result<Value> {
    let id = "mesh.trim_plane";
    let a: Trim = serde_json::from_value(p.clone()).map_err(|e| bad(id, e.to_string()))?;
    mesh_edit::edit(s, id, a.body.as_deref(), |m, f| {
        let (_, normal) = cutting_plane(a.plane, a.normal, a.offset, id)?;
        let plane = equation(f, normal, a.offset);
        m.trim_scan_plane(if a.keep == Keep::Negative { plane.map(|v| -v) } else { plane }).map_err(|e| bad(id, e.to_string()))
    })
}
fn section(s: &mut Session, p: &Value) -> Result<Value> {
    let id = "mesh.section";
    let a: Section = serde_json::from_value(p.clone()).map_err(|e| bad(id, e.to_string()))?;
    let source = mesh_edit::source(s, id, a.body.as_deref())?;
    let (plane, normal) = cutting_plane(a.plane, a.normal, a.offset, id)?;
    let (pts, edges) = source.mesh.section_scan_plane(equation(&source.frame, normal, a.offset)).map_err(|e| bad(id, e.to_string()))?;
    let mut sketch = Sketch::new();
    let mut remap = vec![usize::MAX; pts.len()];
    let curves = edges.len();
    let mut batch = sketch.batch();
    for edge in edges {
        let mut endpoints = [0; 2];
        for (j, i) in edge.into_iter().enumerate() {
            if remap[i] == usize::MAX {
                let [x, y, z] = pts[i];
                let world = apply_point(&source.frame, Vec3::new(x, y, z));
                remap[i] = batch.add_point(plane.to_local(world), None)?;
            }
            endpoints[j] = remap[i];
        }
        batch.add_curve(CurveKind::Line { a: endpoints[0], b: endpoints[1] }, None)?;
    }
    let mut kind = FeatureKind::Sketch { plane: PlaneRef::Custom { plane }, sketch };
    super::component::to_active_frame(s, &mut kind);
    let feature = s.doc_mut().add_feature(kind, a.name.as_deref())?;
    let comp = s.active_component;
    let f = s.doc_mut().feature_mut(feature).ok_or_else(|| bad(id, "section sketch is missing from the document"))?;
    f.component = comp;
    let name = f.name.clone();
    s.refresh();
    if let Some(err) = s.model.result(feature).and_then(|r| r.error.as_ref()) {
        return Err(bad(id, err));
    }
    Ok(json!(SectionResult { feature, name, curves }))
}

#[cfg(test)]
mod tests {
    use crate::{
        Session,
        doc::{Document, FeatureKind, MeshData},
    };
    use serde_json::json;
    #[test]
    fn plane_commands_create_editable_sketch_and_trim_with_undo() {
        let mut doc = Document::default();
        let mesh = MeshData {
            appearance: None,
            name: "Plate".into(),
            positions: vec![0., 0., 0., 4., 0., 0., 4., 4., 0., 0., 4., 0.].into(),
            triangles: vec![0, 1, 2, 0, 2, 3].into(),
            color: None,
            corner_normals: Default::default(),
        };
        doc.add_feature(FeatureKind::MeshImport { file: "plate.obj".into(), meshes: vec![mesh] }, None).unwrap();
        doc.features[0].body_names = vec!["Plate".into()];
        let mut s = Session::new(doc);
        let original = s.doc.to_json();
        let section = s.execute("mesh.section", &json!({"body":"Plate","plane":"YZ","offset":2,"name":"Section A"})).unwrap();
        assert_eq!(section["curves"], 2);
        let feature = s.doc.feature(section["feature"].as_u64().unwrap()).unwrap();
        let FeatureKind::Sketch { sketch, .. } = &feature.kind else { panic!("expected editable sketch") };
        assert_eq!(sketch.points.len(), solidcraft_sketch::Sketch::new().points.len() + 3);
        assert_eq!(sketch.curves.len(), 2);
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.doc.to_json(), original);
        let trim = s.execute("mesh.trim_plane", &json!({"body":"Plate","plane":"XZ","offset":1,"keep":"negative"})).unwrap();
        assert_eq!(trim["triangles_after"], 3);
        let FeatureKind::MeshImport { meshes, .. } = &s.doc.features[0].kind else { panic!("mesh") };
        assert!(meshes[0].positions.as_chunks::<3>().0.iter().all(|p| p[1] <= 1.0));
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.doc.to_json(), original);
    }

    #[test]
    fn automatic_profiles_keep_two_independent_contours() {
        let mut d = Document::default();
        let mut xyz = Vec::new();
        let mut tri = Vec::new();
        for y in [0., 8.] {
            let base = (xyz.len() / 3) as u32;
            for x in [0., 10.] {
                for (dy, z) in [(0., 0.), (4., 0.), (4., 2.), (0., 2.)] {
                    xyz.extend([x, y + dy, z]);
                }
            }
            for i in 0..4 {
                let a = base + i;
                let b = base + (i + 1) % 4;
                tri.extend([a, b, b + 4, a, b + 4, a + 4]);
            }
        }
        let mesh = MeshData {
            appearance: None,
            name: "Twin".into(),
            positions: xyz.into(),
            triangles: tri.into(),
            color: None,
            corner_normals: Default::default(),
        };
        d.add_feature(FeatureKind::MeshImport { file: "twin.obj".into(), meshes: vec![mesh] }, None).unwrap();
        d.features[0].body_names = vec!["Twin".into()];
        let mut s = Session::new(d);
        let r = s.execute("mesh.auto_sections", &json!({"body":"Twin","axis":"x","offsets":[5.,20.],"tolerance":0.01})).unwrap();
        let p = &r["profiles"][0];
        assert_eq!(p["closed_profiles"], 2);
        assert_eq!(p["curves"], 8);
        assert_eq!(r["skipped_offsets"], json!([20.]));
        let state = s.model.state();
        let profiles = &state.sketch(p["feature"].as_u64().unwrap()).unwrap().profiles;
        assert!(profiles.iter().all(|p| (p.area - 8.).abs() < 1e-8));
    }
}
