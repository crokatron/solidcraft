//! Global length-driven triangle remeshing. Only independent, topology-preserving
//! interior collapses are accepted. Conforming bisections can subdivide protected
//! edges without moving their boundary or crease.
use super::{EditResult, edit, parse};
use crate::{Result, Session, params::bad};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
#[cfg(test)]
use solidcore_record::Vec3;
use solidcraft_doc::MeshData;
#[cfg(test)]
use solidcraft_geom::prelude::*;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct Args {
    body: Option<String>,
    target_length: Option<f64>,
}
#[derive(Serialize, JsonSchema)]
pub(super) struct Output {
    #[serde(flatten)]
    edit: EditResult,
    #[serde(flatten)]
    metrics: Metrics,
    remesh_report: String,
}
#[derive(Default, Serialize, JsonSchema)]
struct Metrics {
    target_length_mm: f64,
    mean_edge_before_mm: f64,
    mean_edge_after_mm: f64,
    mean_quality_before: f64,
    mean_quality_after: f64,
    collapsed_edges: usize,
    split_edges: usize,
    refinement_limited: bool,
    appearance_pinned_vertices: usize,
    max_vertex_motion_bound_mm: f64,
}
fn remesh(m: &mut MeshData, length: Option<f64>) -> std::result::Result<Metrics, String> {
    let report = m.remesh_scan(length).map_err(|e| e.to_string())?;
    Ok(Metrics {
        target_length_mm: report.target_length,
        mean_edge_before_mm: report.mean_edge_before,
        mean_edge_after_mm: report.mean_edge_after,
        mean_quality_before: report.quality_before,
        mean_quality_after: report.quality_after,
        collapsed_edges: report.collapsed_edges,
        split_edges: report.split_edges,
        refinement_limited: report.refinement_limited,
        appearance_pinned_vertices: report.appearance_pinned_vertices,
        max_vertex_motion_bound_mm: report.max_vertex_motion_bound,
    })
}
pub(super) fn run(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "mesh.global_remesh";
    let a: Args = parse(p, CMD)?;
    let mut m = Metrics::default();
    let mut r = edit(s, CMD, a.body.as_deref(), |mesh, _| {
        m = remesh(mesh, a.target_length).map_err(|e| bad(CMD, e))?;
        Ok(())
    })?;
    let note = if m.refinement_limited { " Refinement reached the facet budget." } else { "" };
    let report = format!(
        "{} → {} triangles. Target {:.3} mm. Mean edge {:.3} → {:.3} mm. {} edges collapsed, {} split.{}",
        r["triangles_before"],
        r["triangles_after"],
        m.target_length_mm,
        m.mean_edge_before_mm,
        m.mean_edge_after_mm,
        m.collapsed_edges,
        m.split_edges,
        note
    );
    if let (Some(output), Some(metrics)) = (r.as_object_mut(), json!(m).as_object()) {
        output.extend(metrics.clone());
    }
    r["remesh_report"] = json!(report);
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn grid(n: usize, fold: bool) -> MeshData {
        let mut p = vec![];
        let mut f = vec![];
        for j in 0..=n {
            for i in 0..=n {
                let z = if fold { (j as f64 - (n / 2) as f64).max(0.) * 2. } else { 0. };
                p.extend([i as f64, j as f64, z]);
                if i < n && j < n {
                    let a = (j * (n + 1) + i) as u32;
                    let b = a + 1;
                    let c = a + n as u32 + 1;
                    f.extend([a, b, c + 1, a, c + 1, c]);
                }
            }
        }
        MeshData { appearance: None, name: "grid".into(), positions: p.into(), triangles: f.into(), corner_normals: Default::default(), color: None }
    }
    #[test]
    fn global_remeshing_refines_without_cracks_and_keeps_open_boundary_shape() {
        let mut m = grid(2, false);
        let s = remesh(&mut m, Some(0.42)).unwrap();
        assert!(s.split_edges > 0 && s.mean_edge_after_mm < s.mean_edge_before_mm);
        let report = m.scan_topology().unwrap();
        assert_eq!(report.nonmanifold_edges, 0);
        assert_eq!(report.winding_conflicts, 0);
        let ps = m.positions.as_chunks::<3>().0;
        let mut area = 0.;
        for f in m.triangles.as_chunks::<3>().0 {
            let [a, b, c] = f.map(|i| Vec3::from(ps[i as usize]));
            let n = (b - a).cross(c - a);
            assert!(n.z > 0.);
            area += n.z / 2.;
        }
        assert!((area - 4.).abs() < 1e-8);
    }
    #[test]
    fn coarsening_keeps_sharp_feature_vertices_and_manifold_fans() {
        let mut m = grid(14, true);
        let old = m.triangles.len();
        let s = remesh(&mut m, Some(2.25)).unwrap();
        assert!(s.collapsed_edges > 0 && m.triangles.len() < old);
        let pts = m.positions.as_chunks::<3>().0;
        for i in 0..=14 {
            assert!(pts.contains(&[i as f64, 7., 0.]));
        }
        let report = m.scan_topology().unwrap();
        assert_eq!(report.nonmanifold_edges, 0);
        assert_eq!(report.winding_conflicts, 0);
    }
    fn textured_grid(n: usize) -> MeshData {
        use solidcraft_geom::mesh_appearance::{MeshAppearance, MeshMaterial, MeshTexture};
        let mut m = grid(n, false);
        let positions = m.positions.as_chunks::<3>().0;
        let faces = m.triangles.as_chunks::<3>().0;
        let mut materials = Vec::new();
        let mut uvs = Vec::new();
        let mut colors = Vec::new();
        for face in faces {
            let center = face.iter().map(|&i| positions[i as usize][0]).sum::<f64>() / 3.;
            let chart = usize::from(center >= (n / 2) as f64);
            materials.push(chart as u32);
            uvs.push(face.map(|i| {
                let p = positions[i as usize];
                [p[0] as f32 / n as f32 + 2. * chart as f32, p[1] as f32 / n as f32]
            }));
            colors.push([[if chart == 0 { 30 } else { 220 }, 100, 150, 255]; 3]);
        }
        m.appearance = Some(std::sync::Arc::new(MeshAppearance {
            corner_uvs: uvs.into(),
            corner_colors: colors.into(),
            triangle_materials: materials.into(),
            materials: vec![0, 1]
                .into_iter()
                .map(|i| MeshMaterial {
                    name: format!("chart {i}"),
                    base_color: [1.; 4],
                    texture: Some(MeshTexture {
                        name: format!("photo{i}.jpg"),
                        mime_type: "image/jpeg".into(),
                        bytes: vec![255, 216, i, 255, 217].into(),
                    }),
                })
                .collect::<Vec<_>>()
                .into(),
        }));
        m
    }

    #[test]
    fn textured_remesh_and_decimate_preserve_atlas_seams_materials_and_original_images() {
        let original = textured_grid(12);
        let source = original.appearance.as_ref().unwrap();
        for reduce in [false, true] {
            let mut m = original.clone();
            let report = if reduce { m.decimate_scan_report(50).unwrap() } else { m.remesh_scan(Some(0.6)).unwrap() };
            if reduce {
                assert!(report.collapsed_edges > 0);
                assert!(report.achieved_triangles <= 145);
            } else {
                assert!(report.split_edges > 0);
            }
            let a = m.appearance.as_ref().unwrap();
            a.validate(m.triangles.len() / 3).unwrap();
            assert!(std::sync::Arc::ptr_eq(&a.materials, &source.materials));
            let p = m.positions.as_chunks::<3>().0;
            for j in 0..=12 {
                assert!(p.contains(&[6., j as f64, 0.]));
            }
            for (i, face) in m.triangles.as_chunks::<3>().0.iter().enumerate() {
                let chart = a.triangle_materials[i];
                for (j, &v) in face.iter().enumerate() {
                    let point = p[v as usize];
                    assert!((f64::from(a.corner_uvs[i][j][0]) - point[0] / 12. - 2. * f64::from(chart)).abs() < 1e-6);
                    assert!((f64::from(a.corner_uvs[i][j][1]) - point[1] / 12.).abs() < 1e-6);
                    assert_eq!(a.corner_colors[i][j], [if chart == 0 { 30 } else { 220 }, 100, 150, 255]);
                    assert_eq!(point[2], 0.);
                }
            }
            let topology = m.scan_topology().unwrap();
            assert_eq!(topology.nonmanifold_edges, 0);
            assert_eq!(topology.winding_conflicts, 0);
        }
    }
}
