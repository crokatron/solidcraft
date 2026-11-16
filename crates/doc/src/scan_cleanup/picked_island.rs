use super::{MeshData, Result, invalid, shared_vertices};
use rayon::prelude::*;

/// Parent links for triangle components. Roots carry their negative triangle count.
fn root(links: &mut [isize], mut i: usize) -> usize {
    while links[i] >= 0 {
        let parent = links[i] as usize;
        if links[parent] >= 0 {
            links[i] = links[parent];
        }
        i = parent;
    }
    i
}

impl MeshData {
    /// Delete the triangles reachable from the picked triangle across complete shared edges.
    /// A vertex-only contact stays in the mesh. An STL seam with duplicate positions connects.
    pub fn delete_scan_island(&mut self, picked: usize) -> Result<()> {
        let (points, faces) = self.scan_arrays()?;
        let chosen = connected(points, faces, picked, |_| true)?;
        self.retain_scan_region(&chosen, false)
    }
}

/// Compact source triangle indices reachable from the seed across whole eligible edges.
pub(super) fn connected(points: &[[f64; 3]], faces: &[[u32; 3]], picked: usize, eligible: impl Fn(usize) -> bool) -> Result<Vec<u32>> {
    if picked >= faces.len() {
        return Err(invalid("source triangle index out of range"));
    }
    if faces.len() > u32::MAX as usize {
        return Err(invalid("too many source triangles"));
    }
    let mut links = vec![-1isize; faces.len()];
    {
        let canonical = shared_vertices(points);
        let mut edges = Vec::new();
        for (i, face) in faces.iter().enumerate() {
            if !eligible(i) {
                continue;
            }
            let [a, b, c] = face.map(|v| canonical[v as usize]);
            for (u, v) in [(a, b), (b, c), (c, a)] {
                if u != v {
                    edges.push([u.min(v), u.max(v), i as u32]);
                }
            }
        }
        edges.par_sort_unstable_by_key(|e| (e[0], e[1]));
        for group in edges.chunk_by(|a, b| a[..2] == b[..2]) {
            let Some((first, others)) = group.split_first() else { continue };
            for other in others {
                let mut left = root(&mut links, first[2] as usize);
                let mut right = root(&mut links, other[2] as usize);
                if left == right {
                    continue;
                }
                if links[left] > links[right] {
                    std::mem::swap(&mut left, &mut right);
                }
                links[left] += links[right];
                links[right] = left as isize;
            }
        }
    }
    let selected = root(&mut links, picked);
    let mut chosen = Vec::with_capacity(links[selected].unsigned_abs());
    for i in 0..faces.len() {
        if root(&mut links, i) == selected {
            chosen.push(i as u32);
        }
    }
    Ok(chosen)
}

impl MeshData {
    /// Edit a source-facet mask. Inclusive ranges must be ordered, disjoint and in bounds.
    pub fn retain_scan_facets(&mut self, ranges: &[[u32; 2]], isolate: bool) -> Result<()> {
        if ranges.is_empty() {
            return Err(invalid("select at least one facet"));
        }
        let total = self.triangles.len() / 3;
        let mut last = None;
        let mut selected = 0usize;
        for &[a, b] in ranges {
            if b < a || b as usize >= total || last.is_some_and(|n| a <= n) {
                return Err(invalid("facet ranges must be ordered, nonoverlapping and in bounds"));
            }
            selected += b as usize - a as usize + 1;
            last = Some(b);
        }
        let mut ranges = ranges.iter().copied().peekable();
        self.filter_scan_facets(selected, isolate, |i| {
            if ranges.peek().is_some_and(|r| i > r[1]) {
                ranges.next();
            }
            ranges.peek().is_some_and(|r| i >= r[0] && i <= r[1])
        })
    }
    /// Keep only the region, or its complement. Indices come from connected() in ascending order.
    pub(super) fn retain_scan_region(&mut self, chosen: &[u32], isolate: bool) -> Result<()> {
        let mut ids = chosen.iter().copied().peekable();
        self.filter_scan_facets(chosen.len(), isolate, |i| ids.next_if_eq(&i).is_some())
    }
    fn filter_scan_facets(&mut self, selected: usize, isolate: bool, mut belongs: impl FnMut(u32) -> bool) -> Result<()> {
        let (points, faces) = self.scan_arrays()?;
        let remaining = if isolate { selected } else { faces.len() - selected };
        if remaining == 0 {
            return Err(invalid("deleting that region or island would remove the whole mesh"));
        }
        if remaining == faces.len() {
            return Ok(());
        }

        let mut remap = vec![u32::MAX; points.len()];
        let mut positions = Vec::with_capacity(points.len().min(remaining * 3) * 3);
        let mut triangles = Vec::with_capacity(remaining * 3);
        let mut appearance = crate::mesh_edit_appearance::AppearanceBuilder::new(&self.appearance);
        let mut normals = Vec::with_capacity(if self.corner_normals.is_empty() { 0 } else { remaining * 9 });
        for (i, face) in faces.iter().enumerate() {
            if belongs(i as u32) != isolate {
                continue;
            }
            for &v in face {
                let slot = &mut remap[v as usize];
                if *slot == u32::MAX {
                    *slot = (positions.len() / 3) as u32;
                    positions.extend(points[v as usize]);
                }
                triangles.push(*slot);
            }
            appearance.copy_face(i).map_err(|e| invalid(&e))?;
            if !self.corner_normals.is_empty() {
                normals.extend_from_slice(&self.corner_normals[9 * i..9 * i + 9]);
            }
        }
        self.positions = positions.into();
        self.triangles = triangles.into();
        self.corner_normals = normals.into();
        self.appearance = appearance.finish();
        Ok(())
    }
}
