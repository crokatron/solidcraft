//! Wavefront OBJ import: polygon meshes (`v`, `f` with any of `v`, `v/vt`, `v//vn`, `v/vt/vn`,
//! negative indices counting back), one mesh body per object (`o`, else group `g`). Polygons
//! are split into fans. Authored corner normals and smoothing groups are retained. Texture
//! coordinates, material colours and embedded JPEG/PNG maps are retained. The file is
//! untrusted: counts are capped and bad lines become warnings.

use solidcore_record::Vec3;
use solidcraft_geom::mesh_appearance::{MeshAppearance, MeshMaterial, MeshTexture};
use solidcraft_geom::prelude::*;
use std::sync::Arc;

#[derive(Clone, Copy)]
struct Corner {
    vertex: u32,
    normal: Option<u32>,
    uv: Option<u32>,
}

struct Face {
    corners: [Corner; 3],
    smoothing: u32,
    material: u32,
}

fn index(token: &str, len: usize) -> Option<u32> {
    let value: i64 = token.parse().ok()?;
    let len = i64::try_from(len).ok()?;
    let value = if value < 0 { len.checked_add(value)? } else { value.checked_sub(1)? };
    (0..len).contains(&value).then(|| u32::try_from(value).ok()).flatten()
}

use crate::threemf::MeshObject;
use crate::{IoError, Result};

/// Most vertices and triangles we read from one file.
const MAX_VERTICES: usize = 20_000_000;
const MAX_TRIANGLES: usize = 20_000_000;

/// Meshes of an OBJ file and what could not be read.
pub fn read_obj(bytes: &[u8], default_name: &str) -> Result<(Vec<MeshObject>, Vec<String>)> {
    read_obj_with_assets(bytes, default_name, &mut |name| Err(format!("asset `{name}` is unavailable")))
}

/// Import an OBJ and its relative MTL/image assets. Asset bytes are embedded in the result.
pub fn read_obj_with_assets(
    bytes: &[u8],
    default_name: &str,
    load: &mut dyn FnMut(&str) -> std::result::Result<Vec<u8>, String>,
) -> Result<(Vec<MeshObject>, Vec<String>)> {
    let text = String::from_utf8_lossy(bytes);
    let mut verts: Vec<Vec3> = Vec::new();
    // Objects: name and polygons as vertex indices into `verts`.
    let mut objects: Vec<(String, Vec<Face>)> = vec![(default_name.to_string(), Vec::new())];
    let mut warnings: Vec<String> = Vec::new();
    let mut bad_lines = 0usize;
    let mut triangles = 0usize;
    let mut named_by_o = false;
    let mut normals = Vec::<Vec3>::new();
    let mut smoothing = 0u32;
    let mut uvs = Vec::<[f32; 2]>::new();
    let mut colors = Vec::<Option<[u8; 4]>>::new();
    let mut materials = vec![MeshMaterial { name: "Default".into(), base_color: [1.; 4], texture: None }];
    let mut material_names = std::collections::HashMap::<String, u32>::new();
    let mut active_material = 0;
    let mut texture_bytes = 0usize;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if let Some(name) = line.strip_prefix("mtllib ") {
            let path = asset_path(name.trim())?;
            match load(&path) {
                Ok(bytes) => read_materials(&bytes, &path, load, &mut materials, &mut material_names, &mut texture_bytes)?,
                Err(e) => warnings.push(format!("OBJ material library `{path}` could not be read: {e}")),
            }
        }
    }
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let mut it = line.split_whitespace();
        match it.next() {
            Some("v") => {
                let c: Vec<f64> = it.by_ref().take(3).filter_map(|x| x.parse::<f64>().ok()).collect();
                match c[..] {
                    [x, y, z] if x.is_finite() && y.is_finite() && z.is_finite() => verts.push(Vec3::new(x, y, z)),
                    _ => {
                        // Keep the numbering: later faces count vertices by position.
                        verts.push(Vec3::new(f64::NAN, f64::NAN, f64::NAN));
                        bad_lines += 1;
                    }
                }
                let rgb: Vec<f32> = it.take(3).filter_map(|x| x.parse::<f32>().ok()).collect();
                colors.push(match rgb.as_slice() {
                    [r, g, b] if rgb.iter().all(|v| v.is_finite() && (0. ..=1.).contains(v)) => {
                        Some([(*r * 255.).round() as u8, (*g * 255.).round() as u8, (*b * 255.).round() as u8, 255])
                    }
                    _ => None,
                });
                if verts.len() > MAX_VERTICES {
                    return Err(IoError::Invalid(format!("OBJ with more than {MAX_VERTICES} vertices")));
                }
            }
            Some("vt") => {
                let u = it.next().and_then(|v| v.parse::<f32>().ok());
                let v = it.next().and_then(|v| v.parse::<f32>().ok());
                uvs.push(match (u, v) {
                    (Some(u), Some(v)) if u.is_finite() && v.is_finite() => [u, v],
                    _ => [f32::NAN; 2],
                });
                if uvs.len() > MAX_TRIANGLES * 3 {
                    return Err(IoError::Invalid("too many OBJ texture coordinates".into()));
                }
            }
            Some("usemtl") => {
                let name = it.collect::<Vec<_>>().join(" ");
                active_material = if let Some(id) = material_names.get(&name) {
                    *id
                } else {
                    if material_names.len() >= 4096 {
                        return Err(IoError::Invalid("too many OBJ materials".into()));
                    }
                    warnings.push(format!("OBJ material `{name}` is missing; using white"));
                    let id = materials.len() as u32;
                    materials.push(MeshMaterial { name: name.clone(), base_color: [1.; 4], texture: None });
                    material_names.insert(name, id);
                    id
                };
            }
            Some("vn") => {
                let c: Vec<f64> = it.take(3).filter_map(|x| x.parse::<f64>().ok()).collect();
                let normal = match c[..] {
                    [x, y, z] => Vec3::new(x, y, z).normalized().filter(|n| n.is_finite()),
                    _ => None,
                };
                normals.push(normal.unwrap_or(Vec3::new(f64::NAN, f64::NAN, f64::NAN)));
                if normal.is_none() {
                    bad_lines += 1;
                }
                if normals.len() > MAX_VERTICES {
                    return Err(IoError::Invalid(format!("OBJ with more than {MAX_VERTICES} normals")));
                }
            }
            Some("s") => {
                smoothing = match it.next() {
                    Some("off" | "0") | None => 0,
                    Some("on") => 1,
                    Some(v) => v.parse::<u32>().unwrap_or(0),
                };
            }
            Some("o") | Some("g") => {
                let is_o = line.starts_with('o');
                let name = it.collect::<Vec<_>>().join(" ");
                if name.is_empty() || (!is_o && named_by_o) {
                    continue;
                }
                named_by_o |= is_o;
                match objects.last_mut() {
                    // An empty object takes the name instead of starting another.
                    Some((n, tris)) if tris.is_empty() => *n = name,
                    _ => objects.push((name, Vec::new())),
                }
            }
            Some("f") => {
                let corners: Option<Vec<Corner>> = it
                    .map(|tok| {
                        let mut fields = tok.split('/');
                        let vertex = index(fields.next()?, verts.len())?;
                        let uv = fields.next().filter(|v| !v.is_empty()).and_then(|v| index(v, uvs.len()));
                        let normal = fields.next().filter(|v| !v.is_empty()).and_then(|v| index(v, normals.len()));
                        Some(Corner { vertex, normal, uv })
                    })
                    .collect();
                match corners {
                    Some(p) if p.len() >= 3 && p.iter().all(|i| verts.get(i.vertex as usize).is_some_and(|v| v.is_finite())) => {
                        let added = p.len() - 2;
                        triangles = triangles.checked_add(added).ok_or_else(|| IoError::Invalid("OBJ triangle count overflow".into()))?;
                        if triangles > MAX_TRIANGLES {
                            return Err(IoError::Invalid(format!("OBJ with more than {MAX_TRIANGLES} triangles")));
                        }
                        if let Some((_, tris)) = objects.last_mut() {
                            for k in 1..p.len() - 1 {
                                tris.push(Face { corners: [p[0], p[k], p[k + 1]], smoothing, material: active_material });
                            }
                        }
                    }
                    _ => bad_lines += 1,
                }
                if triangles > MAX_TRIANGLES {
                    return Err(IoError::Invalid(format!("OBJ with more than {MAX_TRIANGLES} triangles")));
                }
            }
            _ => {}
        }
    }
    if bad_lines > 0 {
        warnings.push(format!("{bad_lines} vertex or face line(s) could not be read and were left out"));
    }
    let mut out = Vec::new();
    for (name, tris) in objects {
        if tris.is_empty() {
            continue;
        }
        // Each object keeps only its own vertices.
        let mut map = std::collections::HashMap::new();
        let mut positions = Vec::new();
        let mut triangles = Vec::with_capacity(tris.len());
        for t in &tris {
            let mut nt = [0u32; 3];
            for (k, i) in t.corners.iter().enumerate() {
                let next = u32::try_from(positions.len()).map_err(|_| IoError::Invalid("OBJ too large".into()))?;
                nt[k] = *map.entry(i.vertex).or_insert_with(|| {
                    positions.push(verts.get(i.vertex as usize).copied().unwrap_or(Vec3::ZERO));
                    next
                });
            }
            triangles.push(nt);
        }
        let mesh = solidcraft_geom::Mesh { positions, triangles, ..Default::default() };
        let annotated = tris.iter().any(|f| f.smoothing != 0 || f.corners.iter().any(|c| c.normal.is_some()));
        let mut corner_normals = Vec::new();
        if annotated {
            let groups: Vec<u32> = tris.iter().map(|f| f.smoothing).collect();
            corner_normals = mesh.crease_corner_normals(std::f64::consts::PI, Some(&groups)).map_err(IoError::Invalid)?;
            for (f, n) in tris.iter().zip(&mut corner_normals) {
                for (corner, shading) in f.corners.iter().zip(n) {
                    if let Some(normal) = corner.normal.and_then(|k| normals.get(k as usize)).copied().filter(|n| n.is_finite()) {
                        *shading = normal;
                    }
                }
            }
        }
        let has_uv = tris.iter().any(|t| t.corners.iter().any(|c| c.uv.is_some()));
        let has_colors = colors.iter().any(Option::is_some);
        let appearance = if has_uv || has_colors || materials.len() > 1 {
            let mut corner_uvs = Vec::new();
            let mut corner_colors = Vec::new();
            let mut triangle_materials = Vec::new();
            for face in &tris {
                let uv = face.corners.map(|c| c.uv.and_then(|i| uvs.get(i as usize)).copied());
                if materials.get(face.material as usize).is_some_and(|m| m.texture.is_some())
                    && uv.iter().any(|v| v.is_none_or(|v| !v.iter().all(|x| x.is_finite())))
                {
                    return Err(IoError::Invalid("textured OBJ face has missing or invalid texture coordinates".into()));
                }
                if has_uv {
                    corner_uvs.push(uv.map(|v| v.filter(|v| v.iter().all(|x| x.is_finite())).unwrap_or([0.; 2])));
                }
                if has_colors {
                    corner_colors.push(face.corners.map(|c| colors.get(c.vertex as usize).copied().flatten().unwrap_or([255; 4])));
                }
                triangle_materials.push(face.material);
            }
            Some(Arc::new(MeshAppearance {
                corner_uvs: corner_uvs.into(),
                corner_colors: corner_colors.into(),
                triangle_materials: triangle_materials.into(),
                materials: materials.clone().into(),
            }))
        } else {
            None
        };
        out.push(MeshObject { name, positions: mesh.positions, triangles: mesh.triangles, corner_normals, appearance, ..Default::default() });
    }
    if out.is_empty() {
        return Err(IoError::Invalid("OBJ has no faces".into()));
    }
    Ok((out, warnings))
}

/// Restrict external assets to the selected model folder and its descendants.
fn asset_path(name: &str) -> Result<String> {
    let path = name.replace('\\', "/");
    if path.is_empty()
        || path.len() > 4096
        || path.starts_with('/')
        || path.contains(':')
        || path.chars().any(char::is_control)
        || path.split('/').any(|p| p == ".." || p.starts_with('.'))
    {
        return Err(IoError::Invalid("OBJ assets must be relative files inside the model folder".into()));
    }
    Ok(path)
}

fn read_materials(
    bytes: &[u8],
    path: &str,
    load: &mut dyn FnMut(&str) -> std::result::Result<Vec<u8>, String>,
    materials: &mut Vec<MeshMaterial>,
    names: &mut std::collections::HashMap<String, u32>,
    total: &mut usize,
) -> Result<()> {
    if bytes.len() > 4 << 20 {
        return Err(IoError::Invalid("OBJ material library exceeds 4 MiB".into()));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| IoError::Invalid("OBJ material library is not UTF-8".into()))?;
    let mut active = None;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let mut fields = line.split_whitespace();
        match fields.next() {
            Some("newmtl") => {
                let name = fields.collect::<Vec<_>>().join(" ");
                if name.is_empty() || materials.len() >= 4096 {
                    return Err(IoError::Invalid("invalid or excessive OBJ materials".into()));
                }
                let id = materials.len();
                names.insert(name.clone(), id as u32);
                materials.push(MeshMaterial { name, base_color: [1.; 4], texture: None });
                active = Some(id);
            }
            Some("Kd") => {
                if let Some(m) = active.and_then(|i| materials.get_mut(i)) {
                    let c: Vec<f32> = fields.take(3).filter_map(|v| v.parse().ok()).collect();
                    if let [r, g, b] = c.as_slice() {
                        if c.iter().all(|v| v.is_finite() && (0. ..=1.).contains(v)) {
                            m.base_color = [*r, *g, *b, m.base_color[3]];
                        }
                    }
                }
            }
            Some("d" | "Tr") => {
                if let Some(m) = active.and_then(|i| materials.get_mut(i)) {
                    if let Some(v) = fields.next().and_then(|v| v.parse::<f32>().ok()).filter(|v| v.is_finite() && (0. ..=1.).contains(v)) {
                        m.base_color[3] = if line.starts_with("Tr") { 1. - v } else { v };
                    }
                }
            }
            Some("map_Kd") => {
                if let Some(m) = active.and_then(|i| materials.get_mut(i)) {
                    let name = fields.collect::<Vec<_>>().join(" ");
                    if name.starts_with('-') {
                        return Err(IoError::Invalid("OBJ texture map options are not supported; export a plain map_Kd image".into()));
                    }
                    let parent = path.rsplit_once('/').map_or("", |(p, _)| p);
                    let relative = asset_path(&if parent.is_empty() { name.clone() } else { format!("{parent}/{name}") })?;
                    let bytes = load(&relative).map_err(|e| IoError::Invalid(format!("OBJ texture `{relative}` could not be read: {e}")))?;
                    *total = total.checked_add(bytes.len()).ok_or_else(|| IoError::Invalid("OBJ texture size overflow".into()))?;
                    if bytes.len() > 128 << 20 || *total > 256 << 20 {
                        return Err(IoError::Invalid("OBJ texture assets exceed memory limits".into()));
                    }
                    let format = image::guess_format(&bytes).map_err(|e| IoError::Invalid(format!("OBJ texture: {e}")))?;
                    let mime_type = match format {
                        image::ImageFormat::Jpeg => "image/jpeg",
                        image::ImageFormat::Png => "image/png",
                        _ => return Err(IoError::Invalid("OBJ textures must be JPEG or PNG".into())),
                    };
                    let (w, h) = image::ImageReader::with_format(std::io::Cursor::new(&bytes), format)
                        .into_dimensions()
                        .map_err(|e| IoError::Invalid(format!("OBJ texture: {e}")))?;
                    if w == 0 || h == 0 || w > 8192 || h > 8192 || u64::from(w) * u64::from(h) > 64_000_000 {
                        return Err(IoError::Invalid("OBJ texture dimensions exceed 8192 or 64 million pixels".into()));
                    }
                    m.texture = Some(MeshTexture { name, mime_type: mime_type.into(), bytes: bytes.into() });
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texture_uv_seams_materials_and_images_survive_import() {
        let image = image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]));
        let mut encoded = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image).write_to(&mut encoded, image::ImageFormat::Png).unwrap();
        let source=b"mtllib scan.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nv 0 -1 0\nvt 0 0\nvt 1 0\nvt 0 1\nvt 0.5 0.5\nusemtl photo\nf 1/1 2/2 3/3\nf 2/-1 1/2 4/3\n";
        let mut load = |name: &str| match name {
            "scan.mtl" => Ok(b"newmtl photo\nKd 0.5 1 1\nmap_Kd image.png\n".to_vec()),
            "image.png" => Ok(encoded.get_ref().clone()),
            _ => Err("missing".into()),
        };
        let (objects, warnings) = read_obj_with_assets(source, "scan", &mut load).unwrap();
        assert!(warnings.is_empty());
        let a = objects[0].appearance.as_ref().unwrap();
        assert_eq!(objects[0].positions.len(), 4);
        assert_eq!(a.corner_uvs[0], [[0., 0.], [1., 0.], [0., 1.]]);
        assert_eq!(a.corner_uvs[1][0], [0.5, 0.5]);
        assert_eq!(a.triangle_materials.as_ref(), [1, 1]);
        assert_eq!(a.materials[1].base_color, [0.5, 1., 1., 1.]);
        assert_eq!(a.materials[1].texture.as_ref().unwrap().bytes.as_ref(), encoded.get_ref());
        assert!(
            read_obj_with_assets(source, "scan", &mut |name| if name == "scan.mtl" {
                Ok(b"newmtl photo\nmap_Kd ../private.png".to_vec())
            } else {
                panic!("unsafe path loaded")
            })
            .is_err()
        );
        let invalid = String::from_utf8_lossy(source).replace("1/1 2/2 3/3", "1 2 3");
        assert!(read_obj_with_assets(invalid.as_bytes(), "scan", &mut load).is_err());
        let feature = crate::mesh_import_feature_with_assets(source, "scan.obj", 1., &mut load).unwrap();
        let mut doc = solidcraft_doc::Document::new("Textured export");
        doc.add_feature(feature.kind, Some("Scan")).unwrap();
        let mut model = solidcraft_doc::Model::new();
        model.evaluate(&doc);
        let state = model.state();
        assert_eq!(state.bodies.len(), 1);
        let expected = state.bodies[0].mesh_appearance.as_ref().unwrap();
        let files = crate::export_obj(&state, &[], "export.mtl", crate::MeshQuality::default()).unwrap();
        assert_eq!(files.assets.len(), 1);
        assert_eq!(files.assets[0].1.as_ref(), encoded.get_ref());
        let mut reload = |name: &str| {
            if name == "export.mtl" {
                Ok(files.mtl.as_bytes().to_vec())
            } else {
                files.assets.iter().find(|(n, _)| n == name).map(|(_, bytes)| bytes.to_vec()).ok_or_else(|| "missing exported texture".into())
            }
        };
        let (again, warnings) = read_obj_with_assets(files.obj.as_bytes(), "export", &mut reload).unwrap();
        assert!(warnings.is_empty());
        let actual = again[0].appearance.as_ref().unwrap();
        assert_eq!(actual.corner_uvs, expected.corner_uvs);
        for (before, after) in expected.triangle_materials.iter().zip(actual.triangle_materials.iter()) {
            let before = &expected.materials[*before as usize];
            let after = &actual.materials[*after as usize];
            assert_eq!(before.base_color, after.base_color);
            assert_eq!(before.texture.as_ref().unwrap().bytes, after.texture.as_ref().unwrap().bytes);
        }
        assert!(crate::export(&state, &[], crate::Format::StlBinary, "scan").is_err());
    }

    #[test]
    fn authored_corner_normals_and_smoothing_groups_survive_import() {
        let vertices = "v 0 0 0\nv 1 0 0\nv 0 1 0\nv 0 -0.8660254037844386 0.5\n";
        let authored = format!("{vertices}vn 0 0 1\nvn 0 0.5 0.8660254037844386\nf 1//-2 2//-2 3//-2\nf 2//-1 1//-1 4//-1\n");
        let (objects, warnings) = read_obj(authored.as_bytes(), "corners").unwrap();
        assert!(warnings.is_empty());
        assert_eq!(objects[0].positions.len(), 4);
        assert_eq!(objects[0].corner_normals.len(), 2);
        assert_eq!(objects[0].corner_normals[0], [Vec3::Z; 3]);
        assert!((objects[0].corner_normals[1][0] - Vec3::new(0., 0.5, 0.8660254037844386)).len() < 1e-12);
        let grouped = format!("{vertices}s 5\nf 1 2 3\nf 2 1 4\n");
        let (objects, _) = read_obj(grouped.as_bytes(), "smooth").unwrap();
        assert_eq!(objects[0].corner_normals[0][0], objects[0].corner_normals[1][1]);
        assert!(objects[0].corner_normals[0][0].y > 0.2);
        let separated = format!("{vertices}s 5\nf 1 2 3\ns off\nf 2 1 4\n");
        let (objects, _) = read_obj(separated.as_bytes(), "crease").unwrap();
        assert_eq!(objects[0].corner_normals[0][0], Vec3::Z);
        let old = format!("{vertices}f 1 2 3\nf 2 1 4\n");
        assert!(read_obj(old.as_bytes(), "flat").unwrap().0[0].corner_normals.is_empty());
    }

    #[test]
    fn smoothing_groups_ignore_repeated_index_faces_without_losing_normals() {
        let src = "v 0 0 0\nv 1 0 0\nv 0 1 0\nv 0 -0.8660254037844386 0.5\ns 1\nf 1 2 3\nf 1 1 2\nf 2 1 4\n";
        let (objects, warnings) = read_obj(src.as_bytes(), "fold").unwrap();
        assert!(warnings.is_empty());
        assert_eq!(objects.len(), 1);
        let object = &objects[0];
        assert_eq!(object.triangles.len(), 3);
        assert_eq!(object.corner_normals.len(), 3);
        let theta = 15f64.to_radians();
        let shared = Vec3::new(0., theta.sin(), theta.cos());
        assert!((object.corner_normals[0][0] - shared).len() < 1e-12);
        assert_eq!(object.corner_normals[0][0], object.corner_normals[2][1]);
        assert!(object.corner_normals[1].iter().all(|n| n.is_finite() && (n.len() - 1.).abs() < 1e-12));
        let body = solidcraft_kernel::mesh_body_with_normals(&object.positions, &object.triangles, &object.corner_normals).unwrap();
        assert_eq!(body.triangle_mesh().unwrap().1, &[[0, 1, 2], [1, 0, 3]]);
        assert_eq!(body.triangle_corner_normals().unwrap(), &[object.corner_normals[0], object.corner_normals[2]]);
    }

    #[test]
    fn objects_quads_and_negative_indices() {
        let src = "# cube\no Cube\nv 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nv 0 0 1\nv 1 0 1\nv 1 1 1\nv 0 1 1\n\
                   f 1 4 3 2\nf 5 6 7 8\nf 1 2 6 5\nf 2/1 3/2 7/3 6/4\nf 3//1 4//1 8//1 7//1\nf -4 -8 -5 -1\n\
                   o Tri\nv 5 5 5\nv 6 5 5\nv 5 6 5\nf -3 -2 -1\n";
        let (objs, warnings) = read_obj(src.as_bytes(), "x").unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(objs.len(), 2);
        assert_eq!(objs[0].name, "Cube");
        assert_eq!(objs[0].triangles.len(), 12);
        assert_eq!(objs[0].positions.len(), 8);
        assert_eq!(objs[1].name, "Tri");
        assert_eq!(objs[1].triangles.len(), 1);
    }

    #[test]
    fn hostile_obj_never_panics() {
        for src in [
            "",
            "f 1 2 3",
            "v 1 2\nf 1 1 1",
            "v nan 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3",
            "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 99999999999999999999",
            "v 0 0 0\nv 1 0 0\nv 0 1 0\nf -1 -2 -9",
            "o\ng\nf",
            "\u{0}\u{ff}v 1 1 1",
        ] {
            let r = std::panic::catch_unwind(|| read_obj(src.as_bytes(), "x").map(|(o, _)| o.len()));
            assert!(r.is_ok(), "{src:?}");
        }
        assert!(read_obj(b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\nf 1 2 x", "x").unwrap().1.len() == 1);
    }
}
