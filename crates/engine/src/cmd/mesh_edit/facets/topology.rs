use super::{MeshData, Vec3, points};
use solidcraft_geom::Vec3Geom;
use std::sync::Arc;

/// Swap A--B for C--D in the two oriented facets [A,B,C] and [B,A,D].
/// Compare coordinates as well as indices: STL stores independent copies along shared edges.
pub(super) fn flip_edge(mesh: &mut MeshData, picked: usize, corner: usize) -> Result<(), String> {
    if mesh.appearance.is_some() {
        return Err("edge flipping of coloured or textured scans is not supported yet; use split or subdivide to preserve texture seams".into());
    }
    let faces = mesh.triangles.as_chunks::<3>().0;
    let first = faces[picked];
    let a0 = corner;
    let b0 = (corner + 1) % 3;
    let c0 = (corner + 2) % 3;
    let first_pts = points(mesh, first)?;
    let (a, b, c) = (first_pts[a0], first_pts[b0], first_pts[c0]);
    let mut across = None;
    for (i, &face) in faces.iter().enumerate() {
        if i == picked {
            continue;
        }
        let vertices = points(mesh, face)?;
        let (Some(ia), Some(ib)) = (vertices.iter().position(|&p| p == a), vertices.iter().position(|&p| p == b)) else { continue };
        if across.is_some() {
            return Err("cannot flip a non-manifold edge shared by more than two triangles".into());
        }
        if (ib + 1) % 3 != ia {
            return Err("the triangles have inconsistent winding along this edge".into());
        }
        let id = (ia + 1) % 3;
        across = Some((i, ib, id, vertices[id]));
    }
    let (other, b1, d1, d) = across.ok_or("cannot flip a boundary edge")?;
    if c == d {
        return Err("the two triangles have the same opposite vertex".into());
    }
    let second = faces[other];
    // The quadrilateral's oriented area is independent of its diagonal. In the projection
    // on this normal, both diagonals must leave two strictly positive-area triangles.
    let normal = (b - a).cross(c - a) + (a - b).cross(d - b);
    let first_new = (a - c).cross(d - c);
    let second_new = (b - d).cross(c - d);
    let oriented = |n: Vec3| n.dot(normal) > 0.0;
    if ![(b - a).cross(c - a), (a - b).cross(d - b), first_new, second_new].into_iter().all(oriented) {
        return Err("flipping would collapse or invert a triangle".into());
    }
    for (i, &face) in faces.iter().enumerate() {
        if i != picked && i != other {
            let vertices = points(mesh, face)?;
            if vertices.contains(&c) && vertices.contains(&d) {
                return Err("the replacement diagonal already exists".into());
            }
        }
    }
    let changed = [[first[c0], first[a0], second[d1]], [second[d1], second[b1], first[c0]]];
    if !mesh.corner_normals.is_empty() {
        if faces.len().checked_mul(9) != Some(mesh.corner_normals.len()) {
            return Err("invalid mesh normals".into());
        }
        let normal_at = |i: usize, j: usize| mesh.corner_normals.as_chunks::<3>().0[i * 3 + j];
        let updated = [
            [normal_at(picked, c0), normal_at(picked, a0), normal_at(other, d1)],
            [normal_at(other, d1), normal_at(other, b1), normal_at(picked, c0)],
        ];
        // Smooth or artist-authored corner normals travel with their vertex. If both
        // original facets were flat shaded, a non-planar flip needs the new face normals.
        let flat = |i| normal_at(i, 0) == normal_at(i, 1) && normal_at(i, 1) == normal_at(i, 2);
        let updated = if flat(picked) && flat(other) {
            [first_new, second_new].map(|n| {
                let n = n / n.len();
                [[n.x, n.y, n.z]; 3]
            })
        } else {
            updated
        };
        if [picked, other].into_iter().zip(&updated).any(|(i, n)| mesh.corner_normals[9 * i..9 * i + 9] != *n.as_flattened()) {
            let buffer = Arc::make_mut(&mut mesh.corner_normals);
            for (i, n) in [picked, other].into_iter().zip(&updated) {
                buffer[9 * i..9 * i + 9].copy_from_slice(n.as_flattened());
            }
        }
    }
    let triangles = Arc::make_mut(&mut mesh.triangles).as_chunks_mut::<3>().0;
    triangles[picked] = changed[0];
    triangles[other] = changed[1];
    Ok(())
}
