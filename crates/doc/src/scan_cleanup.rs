use crate::{DocError, MeshData, Result};
use solidcore_record::Vec3;
use solidcraft_geom::Vec3Geom;
use std::collections::{HashMap, HashSet};

mod picked_island;
mod planes;
mod region;
mod segment;

fn invalid(message: &str) -> DocError {
    DocError::Invalid(message.into())
}

// Binary STL repeats the same vertex once per incident triangle. Connectivity must follow
// coordinates even when the source indices differ.
pub(super) fn shared_vertices(points: &[[f64; 3]]) -> Vec<u32> {
    let mut unique = HashMap::new();
    points
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let key = p.map(|x| if x == 0.0 { 0 } else { x.to_bits() });
            *unique.entry(key).or_insert(i as u32)
        })
        .collect()
}

/// Edge counts use coincident positions so a triangle-soup STL has the same topology as an indexed mesh.
#[derive(Default, Debug)]
pub struct ScanTopology {
    pub triangles: usize,
    pub boundary_edges: usize,
    pub nonmanifold_edges: usize,
    pub winding_conflicts: usize,
    pub degenerate_triangles: usize,
}

impl MeshData {
    pub fn scan_topology(&self) -> Result<ScanTopology> {
        use rayon::prelude::*;
        let (points, faces) = self.scan_arrays()?;
        let ids = shared_vertices(points);
        let mut report = ScanTopology { triangles: faces.len(), ..Default::default() };
        let mut edges = Vec::with_capacity(self.triangles.len());
        for &face in faces {
            let [a, b, c] = face.map(|id| {
                let [x, y, z] = points[id as usize];
                Vec3::new(x, y, z)
            });
            if (b - a).cross(c - a).len2() == 0.0 {
                report.degenerate_triangles += 1;
                continue;
            }
            let [a, b, c] = face.map(|id| ids[id as usize]);
            for (u, v) in [(a, b), (b, c), (c, a)] {
                // Triangular indexing packs an unordered pair of distinct u32s and its
                // direction into one u64. No per-edge heap allocations on large scans.
                let high = u64::from(u.max(v));
                let low = u64::from(u.min(v));
                edges.push(high * (high - 1) + 2 * low + u64::from(u > v));
            }
        }
        edges.par_sort_unstable();
        for group in edges.chunk_by(|a, b| a / 2 == b / 2) {
            match group {
                [_] => report.boundary_edges += 1,
                [a, b] if a == b => report.winding_conflicts += 1,
                [_, _] => {}
                _ => report.nonmanifold_edges += 1,
            }
        }
        Ok(report)
    }

    pub(super) fn scan_arrays(&self) -> Result<(&[[f64; 3]], &[[u32; 3]])> {
        let (points, trailing_points) = self.positions.as_chunks::<3>();
        let (faces, trailing_faces) = self.triangles.as_chunks::<3>();
        if !trailing_points.is_empty()
            || !trailing_faces.is_empty()
            || faces.is_empty()
            || points.len() > u32::MAX as usize
            || !self.positions.iter().all(|x| x.is_finite())
            || self.triangles.iter().any(|&i| i as usize >= points.len())
            || (!self.corner_normals.is_empty() && faces.len().checked_mul(9) != Some(self.corner_normals.len()))
        {
            return Err(invalid("expected a finite, indexed triangle mesh"));
        }
        if let Some(appearance) = &self.appearance {
            appearance.validate(faces.len()).map_err(|e| invalid(&e))?;
        }
        Ok((points, faces))
    }

    /// Weld coincident points, drop degenerate and duplicate facets, and compact the vertices.
    /// Retain the first orientation and its corner normals when duplicate facets disagree.
    pub fn clean_scan(&mut self) -> Result<()> {
        let (points, faces) = self.scan_arrays()?;
        let remap = shared_vertices(points);
        let mut seen = HashSet::new();
        let mut compact = vec![u32::MAX; points.len()];
        let mut positions = Vec::new();
        let mut triangles = Vec::new();
        let mut normals = Vec::new();
        let mut appearance = crate::mesh_edit_appearance::AppearanceBuilder::new(&self.appearance);
        for (i, &face) in faces.iter().enumerate() {
            let ids = face.map(|id| remap[id as usize]);
            let [a, b, c] = ids.map(|id| {
                let [x, y, z] = points[id as usize];
                Vec3::new(x, y, z)
            });
            if (b - a).cross(c - a).len2() == 0.0 {
                continue;
            }
            let mut key = ids;
            key.sort_unstable();
            if !seen.insert(key) {
                continue;
            }
            for id in ids {
                let slot = &mut compact[id as usize];
                if *slot == u32::MAX {
                    *slot = (positions.len() / 3) as u32;
                    positions.extend(points[id as usize]);
                }
                triangles.push(*slot);
            }
            appearance.copy_face(i).map_err(|e| invalid(&e))?;
            if !self.corner_normals.is_empty() {
                normals.extend_from_slice(&self.corner_normals[9 * i..9 * i + 9]);
            }
        }
        if triangles.is_empty() {
            return Err(invalid("cleanup would remove every triangle"));
        }
        if positions.len() == self.positions.len() && triangles.len() == self.triangles.len() {
            return Ok(());
        }
        self.positions = positions.into();
        self.triangles = triangles.into();
        self.corner_normals = normals.into();
        self.appearance = appearance.finish();
        Ok(())
    }

    /// Drop small vertex-connected islands. Keep the largest even when all fall below the threshold.
    pub fn remove_scan_islands(&mut self, min_triangles: u32) -> Result<()> {
        if min_triangles == 0 {
            return Err(invalid("minimum island size must be at least one triangle"));
        }
        let (points, faces) = self.scan_arrays()?;
        let mut parents: Vec<_> = shared_vertices(points).into_iter().map(|i| i as usize).collect();
        let mut sizes = vec![1usize; points.len()];
        fn root(parents: &mut [usize], mut i: usize) -> usize {
            while parents[i] != i {
                parents[i] = parents[parents[i]];
                i = parents[i];
            }
            i
        }
        for &[a, b, c] in faces {
            for v in [b, c] {
                let mut left = root(&mut parents, a as usize);
                let mut right = root(&mut parents, v as usize);
                if left == right {
                    continue;
                }
                if sizes[left] < sizes[right] {
                    std::mem::swap(&mut left, &mut right);
                }
                parents[right] = left;
                sizes[left] += sizes[right];
            }
        }
        sizes.fill(0);
        for f in faces {
            sizes[root(&mut parents, f[0] as usize)] += 1;
        }
        let largest = sizes.iter().enumerate().max_by_key(|&(_, size)| size).map(|(i, _)| i);
        let keep = |i: usize| Some(i) == largest || sizes[i] >= min_triangles as usize;
        if sizes.iter().enumerate().all(|(i, &n)| n == 0 || keep(i)) {
            return Ok(());
        }
        let mut compact = vec![u32::MAX; points.len()];
        let mut positions = Vec::new();
        let mut triangles = Vec::new();
        let mut normals = Vec::new();
        let mut appearance = crate::mesh_edit_appearance::AppearanceBuilder::new(&self.appearance);
        for (i, &face) in faces.iter().enumerate() {
            if !keep(root(&mut parents, face[0] as usize)) {
                continue;
            }
            for v in face {
                let slot = &mut compact[v as usize];
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

    /// Relax interior vertices toward their one-ring neighbours while pinning open and non-manifold edges.
    pub fn smooth_scan(&mut self, iterations: u32) -> Result<()> {
        if !(1..=20).contains(&iterations) {
            return Err(invalid("smoothing iterations must be between 1 and 20"));
        }
        let (points, faces) = self.scan_arrays()?;
        let shared = shared_vertices(points);
        let mut counts = HashMap::<[u32; 2], u8>::new();
        let mut edges = Vec::new();
        for &face in faces {
            let [a, b, c] = face.map(|i| shared[i as usize]);
            for (i, j) in [(a, b), (b, c), (c, a)] {
                let edge = [i.min(j), i.max(j)];
                let count = counts.entry(edge).or_default();
                if *count == 0 {
                    edges.push((edge[0] as usize, edge[1] as usize));
                }
                *count = count.saturating_add(1);
            }
        }
        let mut pinned = vec![false; points.len()];
        let mut degrees = vec![0usize; points.len()];
        for ([a, b], count) in counts {
            let (a, b) = (a as usize, b as usize);
            if count != 2 {
                pinned[a] = true;
                pinned[b] = true;
            }
            degrees[a] += 1;
            degrees[b] += 1;
        }
        let mut current = points.to_vec();
        let mut sums = vec![[0.0; 3]; points.len()];
        for _ in 0..iterations {
            sums.fill([0.0; 3]);
            for &(a, b) in &edges {
                for k in 0..3 {
                    sums[a][k] += current[b][k];
                    sums[b][k] += current[a][k];
                }
            }
            for (i, p) in current.iter_mut().enumerate() {
                if pinned[i] || degrees[i] == 0 {
                    continue;
                }
                for (v, sum) in p.iter_mut().zip(sums[i]) {
                    *v = 0.5 * *v + 0.5 * (sum / degrees[i] as f64);
                    if !v.is_finite() {
                        return Err(invalid("smoothing exceeded the supported coordinate range"));
                    }
                }
            }
        }
        if current != points {
            for (i, &first) in shared.iter().enumerate() {
                if i != first as usize {
                    current[i] = current[first as usize];
                }
            }
            self.positions = current.into_flattened().into();
            self.corner_normals = Default::default();
        }
        Ok(())
    }

    /// Reduce a scan while preserving material charts and protected geometry.
    pub fn decimate_scan(&mut self, keep_percent: u32) -> Result<()> {
        self.decimate_scan_report(keep_percent).map(|_| ())
    }
}
#[test]
fn topology_counts_shared_stl_vertices_and_separates_edge_defects() {
    let mut mesh = MeshData {
        appearance: None,
        name: "scan".into(),
        positions: vec![0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., -1., 0.].into(),
        triangles: vec![0, 1, 2, 3, 4, 5, 0, 0, 1].into(),
        corner_normals: Default::default(),
        color: None,
    };
    let first = mesh.scan_topology().unwrap();
    assert_eq!(
        (first.triangles, first.degenerate_triangles, first.boundary_edges, first.winding_conflicts, first.nonmanifold_edges),
        (3, 1, 4, 1, 0)
    );
    mesh.triangles = vec![0, 1, 2, 3, 4, 5, 1, 0, 6].into();
    let third = mesh.scan_topology().unwrap();
    assert_eq!((third.boundary_edges, third.winding_conflicts, third.nonmanifold_edges), (6, 0, 1));
}
