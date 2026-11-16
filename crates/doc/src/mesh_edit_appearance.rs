//! Preserve source corner attributes when scan editing changes triangle indexing.
use solidcraft_geom::mesh_appearance::{MeshAppearance, MeshMaterial};
use std::sync::Arc;

/// Builds only attribute arrays that exist on the source; image bytes stay shared.
pub struct AppearanceBuilder {
    source: Option<Arc<MeshAppearance>>,
    uvs: Vec<[[f32; 2]; 3]>,
    colors: Vec<[[u8; 4]; 3]>,
    materials: Vec<u32>,
    neutral: bool,
}

impl AppearanceBuilder {
    pub fn new(source: &Option<Arc<MeshAppearance>>) -> Self {
        Self { source: source.clone(), uvs: Vec::new(), colors: Vec::new(), materials: Vec::new(), neutral: false }
    }

    pub fn copy_face(&mut self, face: usize) -> Result<(), String> {
        let Some(source) = &self.source else { return Ok(()) };
        if !source.corner_uvs.is_empty() {
            self.uvs.push(*source.corner_uvs.get(face).ok_or("missing source texture coordinates")?);
        }
        if !source.corner_colors.is_empty() {
            self.colors.push(*source.corner_colors.get(face).ok_or("missing source corner colours")?);
        }
        if !source.materials.is_empty() {
            self.materials.push(source.triangle_materials.get(face).copied().unwrap_or(0));
        }
        Ok(())
    }

    /// Weights refer to the source triangle's three corners, independently across seams.
    pub fn interpolate_face(&mut self, face: usize, weights: [[f64; 3]; 3]) -> Result<(), String> {
        if weights == [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]] {
            return self.copy_face(face);
        }
        let Some(source) = &self.source else { return Ok(()) };
        if !source.corner_uvs.is_empty() {
            let uv = source.corner_uvs.get(face).ok_or("missing source texture coordinates")?;
            self.uvs.push(weights.map(|w| std::array::from_fn(|axis| (0..3).map(|j| f64::from(uv[j][axis]) * w[j]).sum::<f64>() as f32)));
        }
        if !source.corner_colors.is_empty() {
            let color = source.corner_colors.get(face).ok_or("missing source corner colours")?;
            self.colors
                .push(weights.map(|w| {
                    std::array::from_fn(|axis| (0..3).map(|j| f64::from(color[j][axis]) * w[j]).sum::<f64>().round().clamp(0., 255.) as u8)
                }));
        }
        if !source.materials.is_empty() {
            self.materials.push(source.triangle_materials.get(face).copied().unwrap_or(0));
        }
        Ok(())
    }

    /// New hole caps have no captured photograph. Give them a neutral material while
    /// keeping every original triangle's image mapping intact.
    pub fn neutral_face(&mut self) -> Result<(), String> {
        let Some(source) = &self.source else { return Ok(()) };
        if !source.corner_uvs.is_empty() {
            self.uvs.push([[0.; 2]; 3]);
        }
        if !source.corner_colors.is_empty() {
            self.colors.push([[255; 4]; 3]);
        }
        if !source.materials.is_empty() {
            if source.materials.len() >= 4096 {
                return Err("no material slot available for an untextured repair".into());
            }
            self.materials.push(u32::try_from(source.materials.len()).map_err(|_| "too many mesh materials")?);
            self.neutral = true;
        }
        Ok(())
    }

    pub fn finish(self) -> Option<Arc<MeshAppearance>> {
        self.source.map(|source| {
            let materials = if self.neutral {
                let mut materials = source.materials.to_vec();
                materials.push(MeshMaterial { name: "Untextured repair".into(), base_color: [0.65, 0.65, 0.65, 1.], texture: None });
                materials.into()
            } else {
                source.materials.clone()
            };
            Arc::new(MeshAppearance {
                corner_uvs: self.uvs.into(),
                corner_colors: self.colors.into(),
                triangle_materials: self.materials.into(),
                materials,
            })
        })
    }
}
