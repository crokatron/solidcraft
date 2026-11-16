//! Document adapter for the kernel's editable, chart-aware scan remesher.
use crate::{DocError, MeshData, Result};
use solidcore_ops::scan_remesh::{Attributes, Goal, Scan};
use solidcraft_geom::mesh_appearance::MeshAppearance;
use std::sync::Arc;

pub use solidcore_ops::scan_remesh::Report;

impl MeshData {
    fn resample_scan(&mut self, goal: Goal) -> Result<Report> {
        let mut working = self.clone();
        working.clean_scan()?;
        let (points, faces) = working.scan_arrays()?;
        let mut attributes = Attributes {
            normals: working.corner_normals.as_chunks::<9>().0.iter().map(|n| [[n[0], n[1], n[2]], [n[3], n[4], n[5]], [n[6], n[7], n[8]]]).collect(),
            ..Default::default()
        };
        if let Some(appearance) = &working.appearance {
            attributes.uvs = appearance.corner_uvs.to_vec();
            attributes.colors = appearance.corner_colors.to_vec();
            attributes.materials = appearance.triangle_materials.to_vec();
        }
        let scan = Scan { positions: points.to_vec(), triangles: faces.to_vec(), attributes };
        let (scan, report) = solidcore_ops::scan_remesh::run(scan, goal).map_err(|e| DocError::Invalid(format!("scan remeshing: {e}")))?;
        working.positions = scan.positions.into_flattened().into();
        working.triangles = scan.triangles.into_flattened().into();
        working.corner_normals = scan.attributes.normals.into_iter().flatten().flatten().collect();
        if let Some(source) = &working.appearance {
            let appearance = MeshAppearance {
                corner_uvs: scan.attributes.uvs.into(),
                corner_colors: scan.attributes.colors.into(),
                triangle_materials: scan.attributes.materials.into(),
                materials: source.materials.clone(),
            };
            appearance.validate(working.triangles.len() / 3).map_err(DocError::Invalid)?;
            working.appearance = Some(Arc::new(appearance));
        }
        *self = working;
        Ok(report)
    }

    /// Redistribute scan triangles while keeping authored material charts and image bytes.
    pub fn remesh_scan(&mut self, target_length: Option<f64>) -> Result<Report> {
        self.resample_scan(Goal::Remesh(target_length))
    }

    /// Reduce toward the requested percentage; protected features may prevent reaching it.
    pub fn decimate_scan_report(&mut self, keep_percent: u32) -> Result<Report> {
        if !(1..=80).contains(&keep_percent) {
            return Err(DocError::Invalid("retained triangle percentage must be between 1 and 80".into()));
        }
        let target = (self.triangles.len() / 3).saturating_mul(keep_percent as usize).div_ceil(100).max(1);
        self.resample_scan(Goal::Reduce(target))
    }
}
