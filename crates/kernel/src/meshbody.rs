//! Mesh bodies: triangles without a B-rep (imported 3MF/STL). They render, measure, move and
//! export; B-rep features refuse them.

use solidcraft_geom::prelude::*;
use std::collections::HashMap;

use solidcore_record::Vec3;
use solidcraft_geom::Mesh;

use crate::body::Body;
use crate::{KernelError, Result};

/// Most triangles in one mesh body.
pub const MAX_TRIANGLES: usize = 20_000_000;

/// Indexed triangles with shared vertices, wound counter-clockwise seen from outside.
#[derive(Clone, Debug, Default)]
pub struct TriMesh {
    pub positions: Vec<Vec3>,
    pub triangles: Vec<[u32; 3]>,
    pub corner_normals: Vec<[Vec3; 3]>,
}

impl TriMesh {
    /// Display/measure mesh: corners split so authored seams stay independent.
    pub(crate) fn to_mesh(&self) -> Mesh {
        let mut out = Mesh::default();
        out.positions.reserve(self.triangles.len() * 3);
        out.normals.reserve(self.triangles.len() * 3);
        out.triangles.reserve(self.triangles.len());
        out.tri_face.reserve(self.triangles.len());
        for (index, t) in self.triangles.iter().enumerate() {
            let (Some(a), Some(b), Some(c)) =
                (self.positions.get(t[0] as usize), self.positions.get(t[1] as usize), self.positions.get(t[2] as usize))
            else {
                continue;
            };
            let n = (*b - *a).cross(*c - *a).normalized().unwrap_or(Vec3::Z);
            let base = u32::try_from(out.positions.len()).unwrap_or(u32::MAX);
            out.positions.extend([*a, *b, *c]);
            out.normals.extend(self.corner_normals.get(index).map_or([n; 3], |normals| {
                normals.map(|v| {
                    let v = v.normalized().unwrap_or(n);
                    match v.dot(n).partial_cmp(&0.0) {
                        Some(std::cmp::Ordering::Greater) => v,
                        Some(std::cmp::Ordering::Less) => -v,
                        _ => n,
                    }
                })
            }));
            out.triangles.push([base, base.saturating_add(1), base.saturating_add(2)]);
            out.tri_face.push(0);
        }
        out
    }

    /// Distinct undirected triangle sides.
    pub(crate) fn edge_count(&self) -> usize {
        let mut set = std::collections::HashSet::new();
        for t in &self.triangles {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                set.insert((a.min(b), a.max(b)));
            }
        }
        set.len()
    }

    /// Signed volume ×6.
    fn volume6(&self) -> f64 {
        self.triangles
            .iter()
            .filter_map(|t| Some((self.positions.get(t[0] as usize)?, self.positions.get(t[1] as usize)?, self.positions.get(t[2] as usize)?)))
            .map(|(a, b, c)| a.dot(b.cross(*c)))
            .sum()
    }
}

/// A mesh body from triangles (`positions` in mm). Coincident vertices are merged, degenerate
/// triangles dropped, and a closed mesh with inward winding is turned outward.
pub fn mesh_body(positions: &[Vec3], triangles: &[[u32; 3]]) -> Result<Body> {
    mesh_body_with_normals(positions, triangles, &[])
}

/// A mesh with optional independent, finite nonzero shading normals per triangle corner.
/// Welding and winding repair affect geometric indices; seams never affect connectivity.
pub fn mesh_body_with_normals(positions: &[Vec3], triangles: &[[u32; 3]], normals: &[[Vec3; 3]]) -> Result<Body> {
    let _span = solidcraft_profile::span("kernel", "mesh_body");
    if triangles.len() > MAX_TRIANGLES {
        return Err(KernelError::Invalid(format!("mesh with {} triangles (limit {MAX_TRIANGLES})", triangles.len())));
    }
    if !normals.is_empty() && (normals.len() != triangles.len() || normals.iter().flatten().any(|n| !n.is_finite() || n.normalized().is_none())) {
        return Err(KernelError::Invalid("mesh shading normals must contain three finite nonzero vectors per triangle".into()));
    }
    if positions.iter().any(|p| !(p.is_finite() && p.x.abs() < 1e9 && p.y.abs() < 1e9 && p.z.abs() < 1e9)) {
        return Err(KernelError::Invalid("mesh with non-finite or huge coordinates".into()));
    }
    // Merge vertices that are exactly equal (bit pattern of the coordinates).
    let mut map: HashMap<[u64; 3], u32> = HashMap::with_capacity(positions.len());
    let mut remap: Vec<u32> = Vec::with_capacity(positions.len());
    let mut out = TriMesh::default();
    for p in positions {
        let key = [(p.x + 0.0).to_bits(), (p.y + 0.0).to_bits(), (p.z + 0.0).to_bits()];
        let next = u32::try_from(out.positions.len()).map_err(|_| KernelError::Invalid("too many vertices".into()))?;
        let i = *map.entry(key).or_insert_with(|| {
            out.positions.push(*p);
            next
        });
        remap.push(i);
    }
    for (index, t) in triangles.iter().enumerate() {
        let (Some(&a), Some(&b), Some(&c)) = (remap.get(t[0] as usize), remap.get(t[1] as usize), remap.get(t[2] as usize)) else {
            return Err(KernelError::Invalid("mesh triangle refers to a missing vertex".into()));
        };
        if a == b || b == c || c == a {
            continue;
        }
        out.triangles.push([a, b, c]);
        if let Some(n) = normals.get(index) {
            out.corner_normals.push(*n);
        }
    }
    if out.triangles.is_empty() {
        return Err(KernelError::Invalid("mesh has no triangles".into()));
    }
    let closed = if out.triangles.len() >= 65_536 {
        Some(unify_dense_winding(&mut out))
    } else {
        unify_winding(&mut out);
        None
    };
    if closed != Some(false) && out.volume6() < 0.0 && closed.unwrap_or_else(|| is_closed(&out)) {
        for (i, t) in out.triangles.iter_mut().enumerate() {
            t.swap(1, 2);
            if let Some(n) = out.corner_normals.get_mut(i) {
                n.swap(1, 2);
            }
        }
    }
    Ok(Body::of_mesh(out))
}

/// One packed record per side. Sorting groups incident faces without allocating a
/// vector per edge. The slot includes the original direction for winding repair.
fn unify_dense_winding(mesh: &mut TriMesh) -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    use rayon::prelude::*;
    let records = |(i, t): (usize, &[u32; 3])| {
        std::array::from_fn::<_, 3, _>(|k| {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            ((u64::from(a.min(b)) << 32) | u64::from(a.max(b)), ((3 * i + k) as u32) << 1 | u32::from(a > b))
        })
    };
    #[cfg(not(target_arch = "wasm32"))]
    let mut sides: Vec<_> = mesh.triangles.par_iter().enumerate().flat_map_iter(records).collect();
    #[cfg(target_arch = "wasm32")]
    let mut sides: Vec<_> = mesh.triangles.iter().enumerate().flat_map(records).collect();
    #[cfg(not(target_arch = "wasm32"))]
    sides.par_sort_unstable();
    #[cfg(target_arch = "wasm32")]
    sides.sort_unstable();
    let mut manifold = true;
    let mut consistent = true;
    for group in sides.chunk_by(|a, b| a.0 == b.0) {
        if let [a, b] = group {
            consistent &= (a.1 ^ b.1) & 1 != 0;
        } else {
            manifold = false;
        }
    }
    if consistent {
        return manifold;
    }

    // Each entry gives the adjacent face and whether the two original sides
    // pointed the same way. Non-manifold edges stay disconnected.
    let mut neighbors = vec![[u32::MAX; 3]; mesh.triangles.len()];
    for group in sides.chunk_by(|a, b| a.0 == b.0) {
        if let [a, b] = group {
            let same = 1 ^ ((a.1 ^ b.1) & 1);
            let [a, b] = [a, b].map(|s| (s.1 >> 1) as usize);
            neighbors[a / 3][a % 3] = ((b / 3) as u32) << 1 | same;
            neighbors[b / 3][b % 3] = ((a / 3) as u32) << 1 | same;
        }
    }
    drop(sides);
    let mut flip = vec![2u8; neighbors.len()];
    let mut stack = Vec::new();
    for start in 0..neighbors.len() {
        if flip[start] != 2 {
            continue;
        }
        flip[start] = 0;
        stack.push(start);
        while let Some(i) = stack.pop() {
            for k in 0..3 {
                let side = neighbors[i][if flip[i] == 0 { k } else { 2 - k }];
                if side == u32::MAX {
                    continue;
                }
                let j = (side >> 1) as usize;
                let needed = flip[i] ^ ((side & 1) as u8);
                if flip[j] == 2 {
                    flip[j] = needed;
                    stack.push(j);
                } else if flip[j] != needed {
                    manifold = false;
                }
            }
        }
    }
    for (i, t) in mesh.triangles.iter_mut().enumerate() {
        if flip[i] == 1 {
            t.swap(1, 2);
            if let Some(n) = mesh.corner_normals.get_mut(i) {
                n.swap(1, 2);
            }
        }
    }
    manifold
}

/// Wind every connected piece one way: a triangle that runs a side in the same direction as
/// its neighbour is turned over (files with stray inverted facets). Sides shared by more than
/// two triangles are left alone.
fn unify_winding(m: &mut TriMesh) {
    let mut sides: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (i, t) in m.triangles.iter().enumerate() {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            sides.entry((a.min(b), a.max(b))).or_default().push(i);
        }
    }
    let runs = |t: &[u32; 3], a: u32, b: u32| [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])].contains(&(a, b));
    let mut done = vec![false; m.triangles.len()];
    for start in 0..m.triangles.len() {
        if done[start] {
            continue;
        }
        done[start] = true;
        let mut stack = vec![start];
        while let Some(i) = stack.pop() {
            let Some(t) = m.triangles.get(i).copied() else { continue };
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let Some(list) = sides.get(&(a.min(b), a.max(b))) else { continue };
                if list.len() != 2 {
                    continue;
                }
                for &j in list {
                    if j == i || done[j] {
                        continue;
                    }
                    done[j] = true;
                    if let Some(u) = m.triangles.get_mut(j)
                        && runs(u, a, b)
                    {
                        u.swap(1, 2);
                        if let Some(n) = m.corner_normals.get_mut(j) {
                            n.swap(1, 2);
                        }
                    }
                    stack.push(j);
                }
            }
        }
    }
}

/// Every triangle side is shared by exactly two triangles in opposite directions.
pub(crate) fn is_closed(m: &TriMesh) -> bool {
    let mut count: HashMap<(u32, u32), i32> = HashMap::new();
    for t in &m.triangles {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            *count.entry((a, b)).or_default() += 1;
        }
    }
    count.iter().all(|(&(a, b), &n)| n == 1 && count.get(&(b, a)) == Some(&1))
}

impl Body {
    /// Shared-vertex triangles of a mesh body.
    pub fn triangle_mesh(&self) -> Option<(&[Vec3], &[[u32; 3]])> {
        self.mesh.as_ref().map(|m| (m.positions.as_slice(), m.triangles.as_slice()))
    }
    /// Shading metadata independent of the mesh body's certified geometric triangles.
    pub fn triangle_corner_normals(&self) -> Option<&[[Vec3; 3]]> {
        self.mesh.as_ref().map(|m| m.corner_normals.as_slice())
    }
    /// Does a mesh body enclose a volume (every side shared by two triangles)?
    pub fn is_closed_mesh(&self) -> bool {
        self.mesh.as_ref().is_some_and(|m| is_closed(m))
    }

    /// A mesh body moved by a column-major affine matrix (reflections keep outward winding).
    pub(crate) fn mesh_transformed(&self, m: &[[f64; 4]; 4]) -> Option<Body> {
        let tm = self.mesh.as_ref()?;
        let f = |p: Vec3| {
            Vec3::new(
                m[0][0] * p.x + m[1][0] * p.y + m[2][0] * p.z + m[3][0],
                m[0][1] * p.x + m[1][1] * p.y + m[2][1] * p.z + m[3][1],
                m[0][2] * p.x + m[1][2] * p.y + m[2][2] * p.z + m[3][2],
            )
        };
        let det = m[0][0] * (m[1][1] * m[2][2] - m[2][1] * m[1][2]) - m[1][0] * (m[0][1] * m[2][2] - m[2][1] * m[0][2])
            + m[2][0] * (m[0][1] * m[1][2] - m[1][1] * m[0][2]);
        // Cofactors have the inverse-transpose direction after the determinant's
        // sign is removed. Avoid division by a tiny/zero determinant; display
        // conversion normalizes and falls back to geometry for a collapsed vector.
        let a = Vec3::new(m[0][0], m[0][1], m[0][2]);
        let b = Vec3::new(m[1][0], m[1][1], m[1][2]);
        let c = Vec3::new(m[2][0], m[2][1], m[2][2]);
        let normal = |n: Vec3| {
            let n = (b.cross(c) * n.x + c.cross(a) * n.y + a.cross(b) * n.z) * if det < 0.0 { -1.0 } else { 1.0 };
            let scale = n.x.abs().max(n.y.abs()).max(n.z.abs());
            // Normal directions are independent of the scale's magnitude. Rescale
            // before normalization so a small valid mesh keeps its authored normals.
            if scale > 0.0 && scale.is_finite() { (n / scale).normalized().unwrap_or(n) } else { n }
        };
        let mut out = TriMesh {
            positions: tm.positions.iter().map(|p| f(*p)).collect(),
            triangles: tm.triangles.clone(),
            corner_normals: tm.corner_normals.iter().map(|n| n.map(normal)).collect(),
        };
        if det < 0.0 {
            for (i, t) in out.triangles.iter_mut().enumerate() {
                t.swap(1, 2);
                if let Some(n) = out.corner_normals.get_mut(i) {
                    n.swap(1, 2);
                }
            }
        }
        let mut placed = Body::of_mesh(out);
        placed.color = self.color;
        placed.paint = self.paint.clone();
        placed.splits = self.splits.clone();
        placed.regularization_protected = self.regularization_protected;
        Some(placed)
    }
}

#[cfg(test)]
mod dense_winding_tests {
    use super::*;

    #[test]
    fn dense_and_reference_match_at_seams_boundaries_and_nonmanifold_edges() {
        for faces in [vec![[0, 1, 2], [0, 1, 3], [1, 2, 3], [2, 0, 3]], vec![[0, 1, 2], [0, 1, 3]], vec![[0, 1, 2], [0, 1, 3], [0, 1, 4], [6, 7, 8]]]
        {
            let mut reference = TriMesh { corner_normals: vec![[Vec3::X, Vec3::Y, Vec3::Z]; faces.len()], triangles: faces, ..TriMesh::default() };
            let mut dense = reference.clone();
            unify_winding(&mut reference);
            assert_eq!(unify_dense_winding(&mut dense), is_closed(&reference));
            assert_eq!(reference.triangles, dense.triangles);
            assert_eq!(reference.corner_normals, dense.corner_normals);
        }
    }
}
