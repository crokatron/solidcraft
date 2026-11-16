//! OBJ/MTL sidecar export of authored scan appearance without welding UV seams.
use crate::{IoError, MeshQuality, ObjFiles, Result};
use std::fmt::Write as _;
use std::sync::Arc;
pub(crate) fn export(bodies: &[&solidcraft_doc::ModelBody], mtl_name: &str, quality: MeshQuality) -> Result<ObjFiles> {
    let mut obj = format!("# SolidCraft OBJ export (mm)\nmtllib {mtl_name}\n");
    let mut mtl = String::from("# SolidCraft material library\n");
    let mut assets = Vec::<(String, Arc<[u8]>)>::new();
    let mut vbase = 1usize;
    let mut uvbase = 1usize;
    let prefix = crate::safe_name(mtl_name);
    for (body_index, b) in bodies.iter().enumerate() {
        let mesh = if b.mesh_appearance.is_some() { b.mesh().as_ref().clone() } else { crate::export_mesh_at(&b.body, quality)? };
        let appearance = b.mesh_appearance.as_deref();
        if let Some(a) = appearance {
            a.validate(mesh.triangles.len()).map_err(IoError::Invalid)?;
        }
        let name: String = b.name.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
        let _ = writeln!(obj, "o {name}");
        let colored = appearance.is_some_and(|a| !a.corner_colors.is_empty());
        if colored {
            for (fi, t) in mesh.triangles.iter().enumerate() {
                let colors = appearance.and_then(|a| a.corner_colors.get(fi)).ok_or_else(|| IoError::Invalid("missing scan colours".into()))?;
                for (i, c) in t.iter().zip(colors) {
                    let p = mesh.positions.get(*i as usize).ok_or_else(|| IoError::Invalid("mesh index out of bounds".into()))?;
                    let _ = writeln!(obj, "v {} {} {} {} {} {}", p.x, p.y, p.z, c[0] as f32 / 255., c[1] as f32 / 255., c[2] as f32 / 255.);
                }
            }
        } else {
            for p in &mesh.positions {
                let _ = writeln!(obj, "v {} {} {}", p.x, p.y, p.z);
            }
        }
        let uv = appearance.is_some_and(|a| !a.corner_uvs.is_empty());
        if let Some(a) = appearance {
            for v in a.corner_uvs.iter().flatten() {
                let _ = writeln!(obj, "vt {} {}", v[0], v[1]);
            }
            for (mi, m) in a.materials.iter().enumerate() {
                let _ = writeln!(
                    mtl,
                    "newmtl scan_{body_index}_{mi}\nKd {} {} {}\nd {}",
                    m.base_color[0], m.base_color[1], m.base_color[2], m.base_color[3]
                );
                if let Some(t) = &m.texture {
                    let ext = if t.mime_type == "image/jpeg" { "jpg" } else { "png" };
                    let filename = format!("{prefix}_scan_{body_index}_{mi}.{ext}");
                    let _ = writeln!(mtl, "map_Kd {filename}");
                    assets.push((filename, t.bytes.clone()));
                }
            }
        }
        let fallback = b.body.color().unwrap_or([1.; 3]);
        let _ = writeln!(mtl, "newmtl body_{body_index}\nKd {} {} {}", fallback[0], fallback[1], fallback[2]);
        let mut previous = None;
        for (fi, t) in mesh.triangles.iter().enumerate() {
            let material = appearance.and_then(|a| a.triangle_materials.get(fi).copied().or_else(|| (!a.materials.is_empty()).then_some(0)));
            if fi == 0 || material != previous {
                if let Some(mi) = material {
                    let _ = writeln!(obj, "usemtl scan_{body_index}_{mi}");
                } else {
                    let _ = writeln!(obj, "usemtl body_{body_index}");
                }
                previous = material;
            }
            let ids = if colored { [vbase + fi * 3, vbase + fi * 3 + 1, vbase + fi * 3 + 2] } else { t.map(|i| vbase + i as usize) };
            if uv {
                let u = uvbase + fi * 3;
                let _ = writeln!(obj, "f {}/{} {}/{} {}/{}", ids[0], u, ids[1], u + 1, ids[2], u + 2);
            } else {
                let _ = writeln!(obj, "f {} {} {}", ids[0], ids[1], ids[2]);
            }
        }
        vbase += if colored { mesh.triangles.len() * 3 } else { mesh.positions.len() };
        if uv {
            uvbase += mesh.triangles.len() * 3;
        }
    }
    Ok(ObjFiles { obj, mtl, assets })
}
