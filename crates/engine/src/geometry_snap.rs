//! Exact geometry-derived Move handle placement shared by body and component tools.
use crate::{EngineError, Result, Session};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use solidcore_record::brep::Curve;
use solidcraft_doc::Mat;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GeometryKind {
    Vertex,
    Edge,
    Face,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SnapMode {
    Point,
    Midpoint,
    Center,
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GeometryPick {
    pub body: String,
    pub kind: GeometryKind,
    /// Topology index in the displayed body. A vertex can omit this to resolve nearest exact vertex.
    pub index: Option<usize>,
    /// World-space hint; projected onto exact geometry, never used as the answer itself.
    pub point: [f64; 3],
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct GeometrySnap {
    pub point: [f64; 3],
    pub direction: Option<[f64; 3]>,
    pub label: String,
}
fn err(s: impl ToString) -> EngineError {
    EngineError::Other(s.to_string())
}
fn validate(p: &GeometryPick) -> Result<()> {
    if p.body.is_empty() || p.body.len() > 2048 || p.point.iter().any(|x| !x.is_finite() || x.abs() > 1e9) {
        return Err(err("Invalid geometry pick"));
    }
    Ok(())
}
fn sample(s: solidcraft_kernel::geometry_snap::Sample, label: &str) -> GeometrySnap {
    GeometrySnap { point: s.point, direction: Some(s.direction), label: label.into() }
}
/// Keep the frame origin and apply the shortest rigid rotation aligning its chosen axis.
pub fn align_axis(frame: &Mat, axis: usize, direction: [f64; 3], flip: bool) -> Result<Mat> {
    if !solidcraft_doc::is_placement(frame) {
        return Err(err("Handle frame must be right handed and rigid"));
    }
    solidcraft_kernel::geometry_snap::align_axis(frame, axis, direction, flip).map_err(err)
}
impl Session {
    /// Exact BRep snap. Midpoint is half arc length, solved on the exact curve. Missing topology and mesh bodies are refused.
    pub fn geometry_snap(&self, pick: &GeometryPick, mode: SnapMode) -> Result<GeometrySnap> {
        validate(pick)?;
        let world = self.world_state()?;
        let model = world.bodies.iter().find(|b| b.name == pick.body).ok_or_else(|| err("Selected body is no longer available"))?;
        let body = &model.body;
        if body.is_mesh() {
            return Err(err("Exact handle snaps require CAD edges, faces or vertices; mesh facets are not CAD topology"));
        }
        let record = body.record();
        match pick.kind {
            GeometryKind::Vertex => {
                if mode != SnapMode::Point {
                    return Err(err("A vertex supports point snapping"));
                }
                let index = match pick.index {
                    Some(i) => i,
                    None => {
                        if record.vertices.len() > 100_000 {
                            return Err(err("Select a specific vertex on this body"));
                        }
                        let mut best = None;
                        for (i, v) in record.vertices.iter().enumerate() {
                            let d = solidcraft_kernel::geometry_snap::point_distance(v.point, pick.point).map_err(err)?.value;
                            if best.is_none_or(|(_, old)| d < old) {
                                best = Some((i, d))
                            }
                        }
                        best.map(|(i, _)| i).ok_or_else(|| err("Body has no vertices"))?
                    }
                };
                let point = record.vertices.get(index).ok_or_else(|| err("Selected vertex is no longer available"))?.point;
                Ok(GeometrySnap { point, direction: None, label: "Vertex".into() })
            }
            GeometryKind::Edge => {
                let i = pick.index.ok_or_else(|| err("Select an exact edge"))?;
                let e = record.edges.get(i).ok_or_else(|| err("Selected edge is no longer available"))?;
                if mode == SnapMode::Center {
                    let Curve::Circle { frame, .. } = &e.curve else { return Err(err("Selected edge is not an exact circular arc")) };
                    let direction = solidcraft_kernel::geometry_snap::frame_normal(frame.x_axis, frame.y_axis).map_err(err)?;
                    return Ok(GeometrySnap { point: frame.origin, direction: Some(direction), label: "Circle center".into() });
                }
                let s = body.handle_edge_sample(i, pick.point, mode == SnapMode::Midpoint).map_err(err)?;
                Ok(sample(s, if mode == SnapMode::Midpoint { "Edge midpoint" } else { "Point on edge" }))
            }
            GeometryKind::Face => {
                let i = pick.index.ok_or_else(|| err("Select an exact face"))?;
                let face = record.faces.get(i).ok_or_else(|| err("Selected face is no longer available"))?;
                if mode == SnapMode::Center {
                    use solidcore_record::brep::Surface;
                    let (frame, label) = match &face.surface {
                        Surface::Cylinder { frame, .. } => (frame, "Cylinder face center"),
                        Surface::Cone { frame, .. } => (frame, "Cone face center"),
                        _ => return Err(err("Face center snapping requires a cylindrical or conical face")),
                    };
                    return Ok(sample(body.handle_face_axis_center(i, frame).map_err(err)?, label));
                }
                if mode != SnapMode::Point {
                    return Err(err("Face midpoint is unavailable; choose Center for a cylindrical or conical face"));
                }
                Ok(sample(body.handle_face_sample(i, pick.point).map_err(err)?, "Point on face"))
            }
        }
    }
    /// Resolve one exact straight CAD edge before starting a virtual-corner pick.
    pub fn geometry_straight_edge(&self, p: &GeometryPick) -> Result<[[f64; 3]; 2]> {
        validate(p)?;
        let world = self.world_state()?;
        if p.kind != GeometryKind::Edge {
            return Err(err("Virtual intersection requires two straight CAD edges"));
        }
        let body = &world.bodies.iter().find(|b| b.name == p.body).ok_or_else(|| err("Selected body is no longer available"))?.body;
        if body.is_mesh() {
            return Err(err("Virtual intersection requires CAD edges"));
        }
        let e = body
            .record()
            .edges
            .get(p.index.ok_or_else(|| err("Select an exact edge"))?)
            .ok_or_else(|| err("Selected edge is no longer available"))?;
        match &e.curve {
            Curve::Line { start, end } => Ok([*start, *end]),
            _ => Err(err("Virtual intersection currently supports straight edges only")),
        }
    }
    /// True 3D intersection of the selected straight edges' extended supporting lines.
    /// The first edge supplies the tangent; parallel and skew edges are rejected.
    pub fn geometry_intersection(&self, a: &GeometryPick, b: &GeometryPick) -> Result<GeometrySnap> {
        self.geometry_intersection_transformed(a, None, b, None)
    }
    /// Include transient rigid body previews without adding them to the document.
    pub fn geometry_intersection_transformed(
        &self,
        a: &GeometryPick,
        a_transform: Option<&Mat>,
        b: &GeometryPick,
        b_transform: Option<&Mat>,
    ) -> Result<GeometrySnap> {
        validate(a)?;
        validate(b)?;
        if [a_transform, b_transform].into_iter().flatten().any(|m| !solidcraft_doc::is_placement(m)) {
            return Err(err("Preview transform must be rigid"));
        }
        let placed = |line: [[f64; 3]; 2], transform: Option<&Mat>| {
            line.map(|p| match transform {
                None => p,
                Some(m) => {
                    let p = solidcraft_doc::apply_point(m, solidcore_record::Vec3::new(p[0], p[1], p[2]));
                    [p.x, p.y, p.z]
                }
            })
        };
        Ok(sample(
            solidcraft_kernel::geometry_snap::line_intersection(
                placed(self.geometry_straight_edge(a)?, a_transform),
                placed(self.geometry_straight_edge(b)?, b_transform),
                1e-6,
            )
            .map_err(err)?,
            "Virtual intersection",
        ))
    }
}
