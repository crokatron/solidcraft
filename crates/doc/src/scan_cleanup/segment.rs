use super::{MeshData, Result, invalid, region::face_normal, shared_vertices};
use rayon::prelude::*;
use solidcraft_geom::Vec3Geom;

/// Source facet -> region id (u32::MAX for degenerates), then (representative source facet, size)
/// for each patch, largest first. Manifold-edge crossings respect the local dihedral angle.
/// Total normal deviation from the first facet in a patch is limited to twice that angle.
impl MeshData {
    pub fn segment_scan(&self, degrees: f64) -> Result<(Vec<u32>, Vec<(u32, u32)>)> {
        if !(0.0..=180.0).contains(&degrees) {
            return Err(invalid("normal angle must be 0 to 180 degrees"));
        }
        let (pts, faces) = self.scan_arrays()?;
        if faces.len() >= u32::MAX as usize {
            return Err(invalid("too many scan facets"));
        }
        let normals = faces.par_iter().map(|&f| face_normal(pts, f).unwrap_or(solidcore_record::Vec3::ZERO)).collect::<Vec<_>>();
        let mut neighbors = vec![[u32::MAX; 3]; faces.len()];
        let cos = degrees.to_radians().cos() - 8. * f64::EPSILON;
        {
            let canonical = shared_vertices(pts);
            let mut edges = Vec::with_capacity(self.triangles.len());
            for (i, face) in faces.iter().enumerate() {
                if normals[i] == solidcore_record::Vec3::ZERO {
                    continue;
                }
                let [a, b, c] = face.map(|v| canonical[v as usize]);
                for (u, v) in [(a, b), (b, c), (c, a)] {
                    edges.push([u.min(v), u.max(v), i as u32]);
                }
            }
            edges.par_sort_unstable_by_key(|e| (e[0], e[1]));
            for group in edges.chunk_by(|a, b| a[..2] == b[..2]) {
                let [a, b] = group else { continue };
                let (i, j) = (a[2], b[2]);
                if normals[i as usize].dot(normals[j as usize]) < cos {
                    continue;
                }
                for (from, to) in [(i, j), (j, i)] {
                    if let Some(slot) = neighbors[from as usize].iter_mut().find(|v| **v == u32::MAX) {
                        *slot = to;
                    }
                }
            }
        }
        // Without a fixed reference, locally smooth facets can walk around a small fillet
        // and combine two orthogonal planes. Bound total angular drift across each patch.
        let seed_cos = (2. * degrees).min(180.).to_radians().cos() - 8. * f64::EPSILON;
        let mut ids = vec![u32::MAX; faces.len()];
        let mut groups = Vec::new();
        let mut queue = Vec::new();
        for seed in 0..faces.len() {
            if ids[seed] != u32::MAX || normals[seed] == solidcore_record::Vec3::ZERO {
                continue;
            }
            queue.clear();
            queue.push(seed as u32);
            ids[seed] = seed as u32;
            let mut head = 0;
            while head < queue.len() {
                let from = queue[head];
                head += 1;
                for &to in &neighbors[from as usize] {
                    if to == u32::MAX || ids[to as usize] != u32::MAX {
                        continue;
                    }
                    if normals[to as usize].dot(normals[seed]) >= seed_cos {
                        ids[to as usize] = seed as u32;
                        queue.push(to);
                    }
                }
            }
            groups.push((seed as u32, queue.len() as u32));
        }
        drop(neighbors);
        groups.par_sort_unstable_by_key(|&(seed, size)| (std::cmp::Reverse(size), seed));
        let mut final_ids = vec![u32::MAX; faces.len()];
        for (id, &(seed, _)) in groups.iter().enumerate() {
            final_ids[seed as usize] = id as u32;
        }
        ids.par_iter_mut().for_each(|id| {
            if *id != u32::MAX {
                *id = final_ids[*id as usize]
            }
        });
        Ok((ids, groups))
    }
}
