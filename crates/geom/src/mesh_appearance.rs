//! Authored scan appearance, independent of welded geometry and GPU resources.
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Original image bytes travel with the design, including after moving source files.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeshTexture {
    pub name: String,
    pub mime_type: String,
    pub bytes: Arc<[u8]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeshMaterial {
    pub name: String,
    /// Authored sRGB RGBA multiplier.
    pub base_color: [f32; 4],
    pub texture: Option<MeshTexture>,
}

/// Attributes use triangle corners, so welding positions never joins texture seams.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MeshAppearance {
    #[serde(default, skip_serializing_if = "<[ [[f32; 2]; 3] ]>::is_empty")]
    pub corner_uvs: Arc<[[[f32; 2]; 3]]>,
    #[serde(default, skip_serializing_if = "<[ [[u8; 4]; 3] ]>::is_empty")]
    pub corner_colors: Arc<[[[u8; 4]; 3]]>,
    #[serde(default, skip_serializing_if = "<[u32]>::is_empty")]
    pub triangle_materials: Arc<[u32]>,
    #[serde(default, skip_serializing_if = "<[MeshMaterial]>::is_empty")]
    pub materials: Arc<[MeshMaterial]>,
}

impl PartialEq for MeshAppearance {
    fn eq(&self, other: &Self) -> bool {
        (Arc::ptr_eq(&self.corner_uvs, &other.corner_uvs) || self.corner_uvs == other.corner_uvs)
            && (Arc::ptr_eq(&self.corner_colors, &other.corner_colors) || self.corner_colors == other.corner_colors)
            && (Arc::ptr_eq(&self.triangle_materials, &other.triangle_materials) || self.triangle_materials == other.triangle_materials)
            && (Arc::ptr_eq(&self.materials, &other.materials) || self.materials == other.materials)
    }
}

impl MeshAppearance {
    /// Validate lengths, indices and bounded encoded payloads before evaluating or drawing.
    pub fn validate(&self, faces: usize) -> Result<(), String> {
        if (!self.corner_uvs.is_empty() && self.corner_uvs.len() != faces)
            || (!self.corner_colors.is_empty() && self.corner_colors.len() != faces)
            || (!self.triangle_materials.is_empty() && self.triangle_materials.len() != faces)
        {
            return Err("mesh appearance must contain one entry per triangle".into());
        }
        if self.corner_uvs.iter().flatten().flatten().any(|v| !v.is_finite()) {
            return Err("mesh texture coordinates must be finite".into());
        }
        if self.materials.len() > 4096 || self.triangle_materials.iter().any(|&i| i as usize >= self.materials.len()) {
            return Err("mesh material index or count is invalid".into());
        }
        let mut bytes = 0usize;
        for material in self.materials.iter() {
            if material.base_color.iter().any(|c| !c.is_finite() || !(0.0..=1.0).contains(c)) {
                return Err("mesh material colour must be between zero and one".into());
            }
            if let Some(texture) = &material.texture {
                if self.corner_uvs.len() != faces
                    || texture.bytes.is_empty()
                    || texture.bytes.len() > 128 << 20
                    || !matches!(texture.mime_type.as_str(), "image/png" | "image/jpeg")
                {
                    return Err("mesh texture requires UV coordinates and a bounded PNG or JPEG image".into());
                }
                bytes = bytes.checked_add(texture.bytes.len()).ok_or("mesh texture size overflow")?;
                if bytes > 256 << 20 {
                    return Err("mesh textures exceed 256 MiB".into());
                }
            }
        }
        Ok(())
    }
}
