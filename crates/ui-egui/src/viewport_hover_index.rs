use solidcore_record::Vec3;
use solidcraft_engine::geom::{Mesh, prelude::*};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::{Arc, Mutex, OnceLock, Weak};

/// Highlight geometry follows the displayed tessellation, which can differ from the picking mesh.
#[derive(Default)]
pub(super) struct Cache(Mutex<Vec<Entry>>);
struct Entry {
    mesh: Weak<Mesh>,
    index: Arc<OnceLock<Arc<Index>>>,
}

impl Cache {
    pub(super) fn get(&self, mesh: &Arc<Mesh>) -> Option<Arc<Index>> {
        if mesh.triangles.len().max(mesh.edges.len()) < 4096 {
            return None;
        }
        let mut entries = self.0.lock().ok()?;
        entries.retain(|e| e.mesh.strong_count() != 0);
        if let Some(e) = entries.iter().find(|e| e.mesh.as_ptr() == Arc::as_ptr(mesh)) {
            return e.index.get().cloned();
        }
        // Bound both retained topology and outstanding workers when the pointer crosses parts.
        if entries.len() >= 4 {
            let i = entries.iter().position(|e| e.index.get().is_some())?;
            entries.remove(i);
        }
        let slot = Arc::new(OnceLock::new());
        let source = mesh.clone();
        #[cfg(not(target_arch = "wasm32"))]
        {
            let slot = Arc::clone(&slot);
            std::thread::Builder::new()
                .name("hover-topology".into())
                .spawn(move || {
                    let _ = slot.set(Arc::new(Index::new(&source)));
                })
                .ok()?;
        }
        #[cfg(target_arch = "wasm32")]
        let _ = slot.set(Arc::new(Index::new(&source)));
        let ready = slot.get().cloned();
        entries.push(Entry { mesh: Arc::downgrade(mesh), index: slot });
        ready
    }
}

type PointKey = [i64; 3];
fn point_key(p: Vec3, tol: f64) -> PointKey {
    [p.x, p.y, p.z].map(|v| (v / tol).round() as i64)
}

#[derive(Default)]
struct Face {
    triangles: Vec<Range<usize>>,
    edges: Vec<usize>,
}

pub(super) struct Index {
    faces: HashMap<usize, Face>,
    normals: HashMap<PointKey, Vec<Vec3>>,
    pub(super) diagonal: f64,
}

impl Index {
    pub(super) fn new(m: &Mesh) -> Self {
        let mut faces: HashMap<usize, Face> = HashMap::new();
        for (i, &f) in m.tri_face.iter().take(m.triangles.len()).enumerate() {
            let ranges = &mut faces.entry(f as usize).or_default().triangles;
            match ranges.last_mut() {
                Some(r) if r.end == i => r.end += 1,
                _ => ranges.push(i..i + 1),
            }
        }
        for (ei, fs) in m.edge_faces.iter().enumerate() {
            for &f in fs {
                let edges = &mut faces.entry(f as usize).or_default().edges;
                if edges.last() != Some(&ei) {
                    edges.push(ei);
                }
            }
        }
        let diagonal = m.bounds().diagonal().max(1.0);
        let tol = diagonal * 1e-7;
        let mut normals: HashMap<_, Vec<Vec3>> = m.edges.iter().flatten().map(|p| (point_key(*p, tol), Vec::new())).collect();
        for (p, n) in m.positions.iter().zip(&m.normals) {
            if let Some(ns) = normals.get_mut(&point_key(*p, tol))
                && ns.len() < 8
            {
                ns.push(*n);
            }
        }
        Self { faces, normals, diagonal }
    }

    pub(super) fn patch(&self, mesh: &Mesh, face: usize) -> Vec<usize> {
        let mut found = HashSet::from([face]);
        let mut todo = vec![face];
        while let Some(f) = todo.pop() {
            for &e in self.edges(f) {
                if mesh.seams.get(e) == Some(&true) && mesh.sharp_edges.get(e) != Some(&true) {
                    for &n in mesh.edge_faces.get(e).into_iter().flatten() {
                        if found.insert(n as usize) {
                            todo.push(n as usize);
                        }
                    }
                }
            }
        }
        let mut patch: Vec<_> = found.into_iter().collect();
        patch.sort_unstable();
        patch
    }

    pub(super) fn edges(&self, face: usize) -> &[usize] {
        self.faces.get(&face).map_or(&[], |f| f.edges.as_slice())
    }

    pub(super) fn triangles(&self, face: usize) -> impl Iterator<Item = usize> + '_ {
        self.faces.get(&face).into_iter().flat_map(|f| f.triangles.iter().cloned().flatten())
    }

    pub(super) fn normals(&self, edge: &[Vec3]) -> Vec<[[f32; 3]; 2]> {
        edge.windows(2)
            .map(|w| {
                let points = w.iter().flat_map(|p| self.normals.get(&point_key(*p, self.diagonal * 1e-7)).into_iter().flatten());
                let Some(first) = points.clone().next().copied() else { return [[0.0; 3]; 2] };
                let other = points.copied().min_by(|a, b| a.dot(first).total_cmp(&b.dot(first))).unwrap_or(first);
                [first.to_f32(), other.to_f32()]
            })
            .collect()
    }
}
