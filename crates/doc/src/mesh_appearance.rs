//! Carry authored corner attributes through display welding and winding repair.
use solidcore_record::Vec3;
use solidcraft_geom::mesh_appearance::MeshAppearance;
use std::sync::Arc;

#[cfg(test)]
pub(crate) fn evaluated_appearance(source: &crate::MeshData, display: &solidcraft_geom::Mesh) -> crate::Result<Option<Arc<MeshAppearance>>> {
    evaluated_triangles(source, &display.positions, &display.triangles)
}

/// The kernel's welded triangle list has the same corner order as its display mesh. Reading
/// it directly avoids forcing the expanded display mesh and its crease outlines during an edit.
pub(crate) fn evaluated_triangles(
    source: &crate::MeshData,
    positions: &[Vec3],
    triangles: &[[u32; 3]],
) -> crate::Result<Option<Arc<MeshAppearance>>> {
    let Some(appearance) = &source.appearance else { return Ok(None) };
    appearance.validate(source.triangles.len() / 3).map_err(crate::DocError::Invalid)?;
    let invalid = || crate::DocError::Invalid("cannot map mesh texture corners through display geometry".into());
    let original_positions = source.positions.as_chunks::<3>().0;
    let mut changed: Option<Changed> = None;
    let mut output = 0usize;
    for (face, tri) in source.triangles.as_chunks::<3>().0.iter().enumerate() {
        let original = [
            *original_positions.get(tri[0] as usize).ok_or_else(invalid)?,
            *original_positions.get(tri[1] as usize).ok_or_else(invalid)?,
            *original_positions.get(tri[2] as usize).ok_or_else(invalid)?,
        ];
        // The kernel removes only triangles with coincident welded corners.
        if original[0] == original[1] || original[0] == original[2] || original[1] == original[2] {
            if changed.is_none() {
                changed = Some(Changed::prefix(appearance, face, triangles.len())?);
            }
            continue;
        }
        let target = triangles.get(output).ok_or_else(invalid)?;
        let mut order = [0usize; 3];
        for (corner, &index) in target.iter().enumerate() {
            let point = positions.get(index as usize).ok_or_else(invalid)?;
            order[corner] = original.iter().position(|p| *p == [point.x, point.y, point.z]).ok_or_else(invalid)?;
        }
        if order != [0, 1, 2] && changed.is_none() {
            changed = Some(Changed::prefix(appearance, face, triangles.len())?);
        }
        if let Some(changed) = &mut changed {
            if let Some(values) = appearance.corner_uvs.get(face) {
                changed.uv.push(order.map(|i| values[i]));
            }
            if let Some(values) = appearance.corner_colors.get(face) {
                changed.colors.push(order.map(|i| values[i]));
            }
            if let Some(&value) = appearance.triangle_materials.get(face) {
                changed.materials.push(value);
            }
        }
        output += 1;
    }
    if output != triangles.len() {
        return Err(invalid());
    }
    Ok(Some(match changed {
        None => appearance.clone(),
        Some(changed) => Arc::new(MeshAppearance {
            corner_uvs: changed.uv.into(),
            corner_colors: changed.colors.into(),
            triangle_materials: changed.materials.into(),
            materials: appearance.materials.clone(),
        }),
    }))
}
struct Changed {
    uv: Vec<[[f32; 2]; 3]>,
    colors: Vec<[[u8; 4]; 3]>,
    materials: Vec<u32>,
}
impl Changed {
    fn prefix(appearance: &MeshAppearance, prefix: usize, total: usize) -> crate::Result<Self> {
        fn copy<T: Copy>(source: &[T], prefix: usize, total: usize) -> crate::Result<Vec<T>> {
            let mut out = Vec::new();
            if !source.is_empty() {
                out.try_reserve_exact(total).map_err(|_| crate::DocError::Invalid("cannot reserve remapped scan appearance".into()))?;
                out.extend_from_slice(source.get(..prefix).ok_or_else(|| crate::DocError::Invalid("invalid scan appearance prefix".into()))?);
            }
            Ok(out)
        }
        Ok(Self {
            uv: copy(&appearance.corner_uvs, prefix, total)?,
            colors: copy(&appearance.corner_colors, prefix, total)?,
            materials: copy(&appearance.triangle_materials, prefix, total)?,
        })
    }
}
