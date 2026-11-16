use solidcore_record::Vec3;
use solidcraft_doc::MeshData;
use solidcraft_geom::Vec3Geom;
use solidcraft_geom::compare::closest_on_triangle;
use std::sync::Arc;

mod topology;

pub(super) enum Op {
    Move([f64; 3]),
    Split,
    Subdivide,
    Delete,
    FlipEdge,
    DeleteIsland,
}

fn vertex(m: &MeshData, index: u32) -> Result<Vec3, String> {
    let p = m.positions.as_chunks::<3>().0.get(index as usize).ok_or("triangle refers to a missing vertex")?;
    Ok(Vec3::new(p[0], p[1], p[2]))
}
fn points(m: &MeshData, t: [u32; 3]) -> Result<[Vec3; 3], String> {
    Ok([vertex(m, t[0])?, vertex(m, t[1])?, vertex(m, t[2])?])
}
pub(super) fn choose_triangle(m: &MeshData, p: Vec3) -> Result<usize, String> {
    // Most scan facets lie entirely on one side of the pick. Reject their AABBs before
    // the closest-point calculation. This search runs only when the edit is applied.
    let mut best = f64::INFINITY;
    let mut chosen = None;
    for (i, &tri) in m.triangles.as_chunks::<3>().0.iter().enumerate() {
        let [a, b, c] = points(m, tri)?;
        let outside = |s: f64, u: f64, v: f64, w: f64| (u.min(v).min(w) - s).max(s - u.max(v).max(w)).max(0.);
        let d = Vec3::new(outside(p.x, a.x, b.x, c.x), outside(p.y, a.y, b.y, c.y), outside(p.z, a.z, b.z, c.z));
        if d.len2() >= best || (b - a).cross(c - a).len2() == 0. {
            continue;
        }
        let distance = (closest_on_triangle(p, a, b, c) - p).len2();
        if distance < best {
            best = distance;
            chosen = Some(i);
            if best == 0. {
                break;
            }
        }
    }
    chosen.ok_or_else(|| "no triangle at the pick".into())
}

fn choose_corner(tri: [Vec3; 3], p: Vec3, edge: bool) -> usize {
    let distance = |i: usize| {
        let a = tri[i];
        let ab = tri[(i + 1) % 3] - a;
        let q = if edge && ab.len2() > 0. { a + ab * ((p - a).dot(ab) / ab.len2()).clamp(0., 1.) } else { a };
        (p - q).len2()
    };
    (0..3).min_by(|&a, &b| distance(a).total_cmp(&distance(b))).unwrap_or(0)
}

pub(super) fn apply(m: &mut MeshData, p: Option<Vec3>, index: Option<usize>, corner: Option<usize>, op: Op) -> Result<(), String> {
    if let Some(appearance) = &m.appearance {
        appearance.validate(m.triangles.len() / 3)?;
    }
    if p.is_some_and(|p| !p.is_finite()) || p.is_some() == index.is_some() {
        return Err("give exactly one finite world point or source triangle index".into());
    }
    let idx = match (index, p) {
        (Some(i), _) => i,
        (_, Some(p)) => choose_triangle(m, p)?,
        _ => return Err("missing pick".into()),
    };
    let tri = *m.triangles.as_chunks::<3>().0.get(idx).ok_or("source triangle index out of range")?;
    let pts = points(m, tri)?;
    let at = match &op {
        Op::Move(_) | Op::Split | Op::FlipEdge => match (corner, p) {
            (Some(i), None) if i < 3 => i,
            (None, Some(p)) => choose_corner(pts, p, matches!(op, Op::Split | Op::FlipEdge)),
            _ => return Err("pick a point or give a triangle with corner 0, 1, or 2".into()),
        },
        _ if corner.is_some() => return Err("a face edit takes a triangle without a corner".into()),
        _ => 0,
    };
    if matches!(op, Op::DeleteIsland) {
        return m.delete_scan_island(idx).map_err(|error| error.to_string());
    }
    if (pts[1] - pts[0]).cross(pts[2] - pts[0]).len2() == 0. && !matches!(op, Op::Delete) {
        return Err("the picked triangle is degenerate; clean the scan first".into());
    }
    let a = pts[at];
    let b = pts[(at + 1) % 3];
    if matches!(op, Op::FlipEdge) {
        return topology::flip_edge(m, idx, at);
    }
    if let Op::Move(delta) = op {
        let moved = a + Vec3::new(delta[0], delta[1], delta[2]);
        if moved == a || !moved.is_finite() || [moved.x, moved.y, moved.z].iter().any(|v| v.abs() >= 1e9) {
            return Err("enter a finite nonzero vertex displacement within the model bounds".into());
        }
        // The importer welds equal coordinates even when STL/OBJ stores several source indices.
        // Moving every copy keeps the same connections the displayed mesh has.
        for xyz in Arc::make_mut(&mut m.positions).as_chunks_mut::<3>().0 {
            if *xyz == [a.x, a.y, a.z] {
                *xyz = [moved.x, moved.y, moved.z]
            }
        }
        // Recompute only affected normals. Preserve imported artist normals elsewhere.
        let mut normals = m.corner_normals.clone();
        for (i, &t) in m.triangles.as_chunks::<3>().0.iter().enumerate() {
            let [u, v, w] = points(m, t)?;
            if [u, v, w].contains(&moved) {
                let n = (v - u).cross(w - u).normalized().ok_or("move would collapse a triangle")?;
                if !normals.is_empty() {
                    let group = Arc::make_mut(&mut normals).as_chunks_mut::<9>().0.get_mut(i).ok_or("invalid mesh normals")?;
                    for xyz in group.as_chunks_mut::<3>().0 {
                        *xyz = [n.x, n.y, n.z]
                    }
                }
            }
        }
        m.corner_normals = normals;
        return Ok(());
    }
    let triangles = m.triangles.as_chunks::<3>().0;
    if matches!(op, Op::Delete) && triangles.len() == 1 {
        return Err("the mesh must retain at least one triangle".into());
    }
    let new_vertex = u32::try_from(m.positions.len() / 3).map_err(|_| "mesh is too large")?;
    let mut indices = Vec::with_capacity(m.triangles.len());
    let mut shading = Vec::with_capacity(m.corner_normals.len());
    let mut appearance = solidcraft_doc::mesh_edit_appearance::AppearanceBuilder::new(&m.appearance);
    // Corner index 3 is the new vertex, interpolated independently on each side of a crease.
    // Single-face edits copy the unchanged prefix and suffix as contiguous slices.
    // Only edge splits need to visit the other facets to find shared edge incidences.
    let range = if matches!(op, Op::Split) { 0..triangles.len() } else { idx..idx + 1 };
    let copy_range = |r: std::ops::Range<usize>, indices: &mut Vec<u32>, shading: &mut Vec<f64>| {
        indices.extend_from_slice(&m.triangles[r.start * 3..r.end * 3]);
        if !m.corner_normals.is_empty() {
            shading.extend_from_slice(&m.corner_normals[r.start * 9..r.end * 9]);
        }
    };
    copy_range(0..range.start, &mut indices, &mut shading);
    for face in 0..range.start {
        appearance.copy_face(face)?;
    }
    for i in range.clone() {
        let t = triangles[i];
        let source_normals = m.corner_normals.as_chunks::<9>().0.get(i);
        let mut emit = |corners: [usize; 3], blend: &[usize]| -> Result<(), String> {
            let weights = corners.map(|corner| {
                let mut weights = [0.; 3];
                if corner == 3 {
                    for &j in blend {
                        weights[j] = 1. / blend.len() as f64;
                    }
                } else {
                    weights[corner] = 1.;
                }
                weights
            });
            appearance.interpolate_face(i, weights)?;
            for c in corners {
                indices.push(if c == 3 { new_vertex } else { t[c] });
                if let Some(ns) = source_normals {
                    let normal = |j: usize| Vec3::new(ns[3 * j], ns[3 * j + 1], ns[3 * j + 2]);
                    let n = if c == 3 {
                        blend.iter().fold(Vec3::ZERO, |s, &j| s + normal(j)).normalized().unwrap_or_else(|| normal(blend[0]))
                    } else {
                        normal(c)
                    };
                    shading.extend([n.x, n.y, n.z]);
                }
            }
            Ok(())
        };
        match op {
            Op::Delete if i == idx => {}
            Op::Subdivide if i == idx => {
                for j in 0..3 {
                    emit([j, (j + 1) % 3, 3], &[0, 1, 2])?;
                }
            }
            Op::Split => {
                let verts = points(m, t)?;
                let on = (0..3).find(|&j| {
                    let pair = [verts[j], verts[(j + 1) % 3]];
                    pair == [a, b] || pair == [b, a]
                });
                if let Some(j) = on {
                    let k = (j + 1) % 3;
                    let other = (j + 2) % 3;
                    emit([j, 3, other], &[j, k])?;
                    emit([3, k, other], &[j, k])?;
                } else {
                    emit([0, 1, 2], &[])?
                }
            }
            _ => emit([0, 1, 2], &[])?,
        }
    }
    copy_range(range.end..triangles.len(), &mut indices, &mut shading);
    for face in range.end..triangles.len() {
        appearance.copy_face(face)?;
    }
    let new_point = match op {
        Op::Split => Some((a + b) * 0.5),
        Op::Subdivide => Some((pts[0] + pts[1] + pts[2]) * (1. / 3.)),
        _ => None,
    };
    if let Some(v) = new_point {
        let mut positions = m.positions.to_vec();
        positions.extend([v.x, v.y, v.z]);
        m.positions = positions.into();
    }
    m.triangles = indices.into();
    m.corner_normals = shading.into();
    m.appearance = appearance.finish();
    Ok(())
}
