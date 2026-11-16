//! Compact authored mesh attributes; images remain embedded in the material payload.
use crate::{IoError, Result};
use solidcraft_geom::mesh_appearance::{MeshAppearance, MeshMaterial};
use std::sync::Arc;
fn invalid(s: impl Into<String>) -> IoError {
    IoError::Invalid(s.into())
}
pub fn encode(a: &MeshAppearance, faces: usize) -> Result<Vec<u8>> {
    a.validate(faces).map_err(invalid)?;
    let materials = serde_json::to_vec(&a.materials).map_err(|e| invalid(e.to_string()))?;
    let size = 40usize
        .checked_add(a.corner_uvs.len().saturating_mul(24))
        .and_then(|n| n.checked_add(a.corner_colors.len().saturating_mul(12)))
        .and_then(|n| n.checked_add(a.triangle_materials.len().saturating_mul(4)))
        .and_then(|n| n.checked_add(materials.len()))
        .ok_or_else(|| invalid("mesh appearance size overflow"))?;
    if size > 1024 << 20 {
        return Err(invalid("mesh appearance exceeds 1 GiB"));
    }
    let mut out = Vec::new();
    out.try_reserve_exact(size).map_err(|_| invalid("could not reserve mesh appearance"))?;
    out.extend_from_slice(b"SCAPPR01");
    for n in [a.corner_uvs.len(), a.corner_colors.len(), a.triangle_materials.len(), materials.len()] {
        out.extend_from_slice(&(n as u64).to_le_bytes());
    }
    for v in a.corner_uvs.iter().flatten().flatten() {
        out.extend_from_slice(&v.to_le_bytes());
    }
    for c in a.corner_colors.iter().flatten() {
        out.extend_from_slice(c);
    }
    for m in a.triangle_materials.iter() {
        out.extend_from_slice(&m.to_le_bytes());
    }
    out.extend_from_slice(&materials);
    Ok(out)
}
pub fn decode(bytes: &[u8], faces: usize) -> Result<Arc<MeshAppearance>> {
    if bytes.get(..8) != Some(b"SCAPPR01") || bytes.len() > 1024 << 20 {
        return Err(invalid("invalid mesh appearance blob"));
    }
    let count = |start| -> Result<usize> {
        let a: [u8; 8] =
            bytes.get(start..start + 8).ok_or_else(|| invalid("short mesh appearance"))?.try_into().map_err(|_| invalid("short mesh appearance"))?;
        usize::try_from(u64::from_le_bytes(a)).map_err(|_| invalid("mesh appearance size overflow"))
    };
    let (uv, colors, indices, material_bytes) = (count(8)?, count(16)?, count(24)?, count(32)?);
    if [uv, colors, indices].iter().any(|&n| n != 0 && n != faces) {
        return Err(invalid("mesh appearance triangle count mismatch"));
    }
    let color_start = uv.checked_mul(24).and_then(|n| n.checked_add(40)).ok_or_else(|| invalid("mesh appearance size overflow"))?;
    let index_start = colors.checked_mul(12).and_then(|n| n.checked_add(color_start)).ok_or_else(|| invalid("mesh appearance size overflow"))?;
    let material_start = indices.checked_mul(4).and_then(|n| n.checked_add(index_start)).ok_or_else(|| invalid("mesh appearance size overflow"))?;
    if material_start.checked_add(material_bytes) != Some(bytes.len()) {
        return Err(invalid("mesh appearance size mismatch"));
    }
    let mut corner_uvs = Vec::new();
    corner_uvs.try_reserve_exact(uv).map_err(|_| invalid("could not reserve mesh UVs"))?;
    for face in bytes.get(40..color_start).ok_or_else(|| invalid("short mesh UVs"))?.chunks_exact(24) {
        let mut v = [[0.; 2]; 3];
        for (slot, raw) in v.iter_mut().flatten().zip(face.chunks_exact(4)) {
            let a: [u8; 4] = raw.try_into().map_err(|_| invalid("short mesh UV"))?;
            *slot = f32::from_le_bytes(a);
        }
        corner_uvs.push(v);
    }
    let mut corner_colors = Vec::new();
    corner_colors.try_reserve_exact(colors).map_err(|_| invalid("could not reserve mesh colours"))?;
    for face in bytes.get(color_start..index_start).ok_or_else(|| invalid("short mesh colours"))?.chunks_exact(12) {
        let mut v = [[0; 4]; 3];
        for (slot, raw) in v.iter_mut().zip(face.chunks_exact(4)) {
            slot.copy_from_slice(raw);
        }
        corner_colors.push(v);
    }
    let mut triangle_materials = Vec::new();
    triangle_materials.try_reserve_exact(indices).map_err(|_| invalid("could not reserve mesh materials"))?;
    for raw in bytes.get(index_start..material_start).ok_or_else(|| invalid("short mesh materials"))?.chunks_exact(4) {
        triangle_materials.push(u32::from_le_bytes(raw.try_into().map_err(|_| invalid("short mesh material"))?));
    }
    let materials: Arc<[MeshMaterial]> =
        serde_json::from_slice(bytes.get(material_start..).ok_or_else(|| invalid("short material payload"))?).map_err(|e| invalid(e.to_string()))?;
    let a = MeshAppearance {
        corner_uvs: corner_uvs.into(),
        corner_colors: corner_colors.into(),
        triangle_materials: triangle_materials.into(),
        materials,
    };
    a.validate(faces).map_err(invalid)?;
    Ok(Arc::new(a))
}
