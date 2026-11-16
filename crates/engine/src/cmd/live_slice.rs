//! Display geometry for the independent planar slice viewer and its cloud tracing sketch.
use super::{CommandSpec, mesh_edit};
use crate::{
    Result, Session,
    params::bad,
    schema::{TypedContract, output_schema_for},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use solidcore_record::{Plane, Vec2, Vec3};
use solidcraft_doc::{FeatureKind, PlaneRef, apply_point};
use solidcraft_sketch::{CurveKind, Sketch};
use std::collections::HashSet;

const ID: &str = "scan.live_slice";
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Trace {
    cloud: u64,
    axis: String,
    offset: f64,
    thickness: f64,
    points: Option<Vec<[f64; 2]>>,
    closed: Option<bool>,
    name: Option<String>,
}
#[derive(Serialize, JsonSchema)]
struct Created {
    feature: u64,
    reference_points: usize,
    curves: usize,
    name: String,
}
pub(super) static COMMANDS:&[CommandSpec]=&[
    CommandSpec::new(ID,"Live Slice",crate::checked!("scan.live_slice",available)).icon("section").noundo().contract(&TypedContract::<Empty>::new()).params("Available world-space scan mesh and raw point-cloud slice sources."),
    CommandSpec::new("pointcloud.trace_slice","Create Slice Sketch",crate::checked!("pointcloud.trace_slice",trace)).icon("sketch").contract(&TypedContract::<Trace>::new()).output(output_schema_for::<Created>).params("Create an editable sketch of a point-cloud slab at offset in mm, with up to 512 projected reference points. Optional picked 2D points form a polyline."),
];
fn available(s: &mut Session, _: &Value) -> Result<Value> {
    let state = s.world_state()?;
    Ok(
        json!({"meshes":state.bodies.iter().filter(|b|b.body.is_mesh()).map(|b|&b.name).collect::<Vec<_>>(),"clouds":crate::view::shown_point_clouds(s).map(|c|json!({"id":c.id,"name":c.name})).collect::<Vec<_>>()}),
    )
}
#[derive(Default)]
pub struct SliceGeometry {
    pub bounds: [[f64; 3]; 2],
    pub offset: f64,
    pub thickness: f64,
    pub context: Vec<[f64; 3]>,
    pub slab: Vec<[f64; 3]>,
    pub segments: Vec<[[f64; 3]; 2]>,
    pub count: usize,
    pub segment_count: usize,
}
fn xyz(p: Vec3) -> [f64; 3] {
    [p.x, p.y, p.z]
}
fn axis_num(a: &str) -> Result<usize> {
    ["x", "y", "z"].iter().position(|k| *k == a).ok_or_else(|| bad(ID, "axis must be x, y or z"))
}
fn uv(a: usize) -> [usize; 2] {
    [[1, 2], [0, 2], [0, 1]][a]
}
impl Session {
    pub fn scan_live_slice(
        &self,
        body: Option<&str>,
        cloud: Option<u64>,
        axis: usize,
        offset: Option<f64>,
        thickness: Option<f64>,
    ) -> Result<SliceGeometry> {
        if axis > 2 || offset.is_some_and(|x| !x.is_finite()) || thickness.is_some_and(|x| !x.is_finite() || x <= 0.) {
            return Err(bad(ID, "enter a valid axis, offset and positive slab thickness"));
        }
        if body.is_some() == cloud.is_some() {
            return Err(bad(ID, "choose one mesh or cloud"));
        }
        let src = body.map(|b| mesh_edit::source(self, ID, Some(b))).transpose()?;
        let cl = cloud
            .map(|id| {
                crate::view::shown_point_clouds(self).find(|c| c.id == id).ok_or_else(|| bad(ID, "the selected cloud is hidden or no longer exists"))
            })
            .transpose()?;
        let filter = cl.map(mesh_edit::cloud_sample_shown);
        let ps = src.as_ref().map(|s| (s.mesh.positions.as_chunks::<3>().0, &s.frame));
        let n = ps.map_or_else(|| cl.map_or(0, |c| c.len()), |(p, _)| p.len());
        let point = |i: usize| -> Option<[f64; 3]> {
            if let Some((points, frame)) = ps {
                return points.get(i).map(|&p| xyz(apply_point(frame, Vec3::from(p))));
            }
            let p = cl?.world_point(i)?;
            filter.as_ref().is_some_and(|shown| shown(i, p)).then(|| xyz(p))
        };
        let mut g = SliceGeometry { bounds: [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]], ..Default::default() };
        for i in 0..n {
            let Some(p) = point(i) else {
                continue;
            };
            for j in 0..3 {
                g.bounds[0][j] = g.bounds[0][j].min(p[j]);
                g.bounds[1][j] = g.bounds[1][j].max(p[j]);
            }
        }
        if !g.bounds[0][axis].is_finite() {
            return Err(bad(ID, "the source contains no visible points"));
        }
        let lo = g.bounds[0][axis];
        let hi = g.bounds[1][axis];
        g.offset = offset.unwrap_or((lo + hi) * 0.5);
        g.thickness = thickness.unwrap_or(((hi - lo) * 0.02).max(0.1));
        let inside = |p: [f64; 3]| (p[axis] - g.offset).abs() <= 0.5 * g.thickness;
        let stride = n.div_ceil(8000);
        for i in (0..n).step_by(stride) {
            if let Some(p) = point(i) {
                g.context.push(p);
            }
        }
        // Reservoir sampling is deterministic, bounded and independent of input ordering density.
        let mut rng = 123456789_u64;
        for i in 0..n {
            let Some(p) = point(i) else {
                continue;
            };
            if !inside(p) {
                continue;
            }
            g.count += 1;
            if g.slab.len() < 16000 {
                g.slab.push(p);
            } else {
                rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
                let j = ((u128::from(rng) * g.count as u128) >> 64) as usize;
                if j < g.slab.len() {
                    g.slab[j] = p;
                }
            }
        }
        if let Some(s) = &src {
            let coeff = s.frame.map(|col| col[axis]);
            let plane = [coeff[0], coeff[1], coeff[2], coeff[3] - g.offset];
            if let Ok((pts, edges)) = s.mesh.section_scan_plane(plane) {
                g.segment_count = edges.len();
                let step = edges.len().div_ceil(20000).max(1);
                for e in edges.iter().step_by(step) {
                    g.segments.push(e.map(|i| xyz(apply_point(&s.frame, Vec3::from(pts[i])))));
                }
            }
        }
        Ok(g)
    }
}
fn trace(s: &mut Session, p: &Value) -> Result<Value> {
    const TRACE: &str = "pointcloud.trace_slice";
    let a: Trace = serde_json::from_value(p.clone()).map_err(|e| bad(TRACE, e.to_string()))?;
    let axis = axis_num(&a.axis)?;
    let g = s.scan_live_slice(None, Some(a.cloud), axis, Some(a.offset), Some(a.thickness))?;
    if g.count == 0 {
        return Err(bad(TRACE, "no points fall inside this slab"));
    }
    let [u, v] = uv(axis);
    let unit = [Vec3::X, Vec3::Y, Vec3::Z];
    let plane = Plane { origin: unit[axis] * a.offset, x: unit[u], y: unit[v] };
    let path = a.points.unwrap_or_default();
    if path.len() == 1 || path.len() > 8192 || path.iter().flatten().any(|v| !v.is_finite()) {
        return Err(bad(TRACE, "trace at least two distinct points, or leave the path empty for a reference sketch"));
    }
    let mut sketch = Sketch::new();
    let mut batch = sketch.batch();
    let mut seen = HashSet::new();
    let mut refs = 0;
    for p in g.slab.iter().step_by(g.slab.len().div_ceil(512).max(1)) {
        let q = Vec2::new(p[u], p[v]);
        if seen.insert([q.x.to_bits(), q.y.to_bits()]) {
            batch.add_point(q, None)?;
            refs += 1;
        }
    }
    let mut ids = vec![];
    for q in path {
        ids.push(batch.add_point(Vec2::new(q[0], q[1]), None)?);
    }
    if a.closed == Some(true) && ids.len() > 2 {
        ids.push(ids[0]);
    }
    for pair in ids.windows(2) {
        batch.add_curve(CurveKind::Line { a: pair[0], b: pair[1] }, None)?;
    }
    let curves = ids.len().saturating_sub(1);
    let mut kind = FeatureKind::Sketch { plane: PlaneRef::Custom { plane }, sketch };
    super::component::to_active_frame(s, &mut kind);
    let feature = s.doc_mut().add_feature(kind, a.name.as_deref())?;
    let component = s.active_component;
    let f = s.doc_mut().feature_mut(feature).ok_or_else(|| bad(TRACE, "new sketch could not be resolved"))?;
    f.component = component;
    let name = f.name.clone();
    s.refresh();
    Ok(json!(Created { feature, reference_points: refs, curves, name }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_cloud_slice_respects_display_filters_and_refuses_stale_sources() {
        let mut session = Session::default();
        assert!(session.scan_live_slice(None, Some(919), 0, None, None).is_err());
        let source = std::env::temp_dir().join(format!("solidcraft-live-slice-{}.xyz", std::process::id()));
        std::fs::write(&source, "0 0 0\n1 1 0\n2 1 2\n").unwrap();
        let cloud = session.execute("pointcloud.insert", &json!({"path":source,"units":"mm"})).unwrap()["cloud"].as_u64().unwrap();
        let _ = std::fs::remove_file(source);
        session.execute("pointcloud.edit", &json!({"cloud":cloud,"filters":[{"property":"x","range":[0.5,1.5]}]})).unwrap();
        let slice = session.scan_live_slice(None, Some(cloud), 2, Some(0.), Some(0.1)).unwrap();
        assert_eq!(slice.count, 1);
        assert_eq!(slice.slab[0], [1., 1., 0.]);
        let result = session
            .execute(
                "pointcloud.trace_slice",
                &json!({"cloud":cloud,"axis":"z","offset":0,"thickness":0.1,"points":[[0,0],[1,0],[1,1]],"closed":true}),
            )
            .unwrap();
        assert_eq!(result["reference_points"], 1);
        assert_eq!(result["curves"], 3);
        session.execute("pointcloud.edit", &json!({"cloud":cloud,"visible":false})).unwrap();
        assert!(session.scan_live_slice(None, Some(cloud), 2, None, None).is_err());
    }
}
