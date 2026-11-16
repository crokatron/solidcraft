use super::invalid;
use crate::{MeshData, Result};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};

type Point = [f64; 3];
type Key = [u64; 3];
fn key(p: Point) -> Key {
    p.map(|v| if v == 0.0 { 0 } else { v.to_bits() })
}

/// A source vertex or the intersection of an original triangle side with the plane.
#[derive(Clone, Copy)]
enum Corner {
    Vertex(usize),
    Cut(usize, usize),
}

fn polygon(face: [u32; 3], dist: &[f64], out: &mut [Corner; 4]) -> usize {
    let mut n = 0;
    for i in 0..3 {
        let j = (i + 1) % 3;
        let (a, b) = (dist[face[i] as usize], dist[face[j] as usize]);
        if a >= 0.0 {
            out[n] = Corner::Vertex(i);
            n += 1;
        }
        if (a < 0.0 && b > 0.0) || (a > 0.0 && b < 0.0) {
            out[n] = Corner::Cut(i, j);
            n += 1;
        }
    }
    n
}

fn interpolate(a: Point, b: Point, t: f64) -> Point {
    std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t)
}
fn cut(points: &[Point], ds: &[f64], a: u32, b: u32) -> (Point, f64) {
    let (pa, pb) = (points[a as usize], points[b as usize]);
    let (da, db) = (ds[a as usize], ds[b as usize]);
    // Identical bit patterns for opposite windings and triangles duplicated across STL seams.
    if key(pa) <= key(pb) {
        let t = da / (da - db);
        (interpolate(pa, pb, t), t)
    } else {
        let t = db / (db - da);
        (interpolate(pb, pa, t), 1.0 - t)
    }
}

fn distances(points: &[Point], plane: [f64; 4]) -> Vec<f64> {
    let [x, y, z, c] = plane;
    points
        .par_iter()
        .map(|p| {
            let v = x * p[0] + y * p[1] + z * p[2] + c;
            let eps = 8.0 * f64::EPSILON * ((x * p[0]).abs() + (y * p[1]).abs() + (z * p[2]).abs() + c.abs()).max(1.0);
            if v.abs() <= eps { 0.0 } else { v }
        })
        .collect()
}

impl MeshData {
    /// Clip against the nonnegative half-space of a plane in mesh-local coordinates.
    pub fn trim_scan_plane(&mut self, plane: [f64; 4]) -> Result<()> {
        let (p, faces) = self.scan_arrays()?;
        let d = distances(p, plane);
        let mut remap = vec![u32::MAX; p.len()];
        let mut cuts = HashMap::new();
        let mut positions = Vec::new();
        let mut indices = Vec::new();
        let mut normals = Vec::new();
        let mut appearance = crate::mesh_edit_appearance::AppearanceBuilder::new(&self.appearance);
        let shading = self.corner_normals.as_chunks::<3>().0;
        let mut removed = false;
        for (f, &face) in faces.iter().enumerate() {
            if face.iter().any(|&i| d[i as usize] < 0.0) {
                removed = true;
            }
            let mut corners = [Corner::Vertex(0); 4];
            let n = polygon(face, &d, &mut corners);
            if n < 3 {
                continue;
            }
            let mut verts = [0; 4];
            let mut ns = [[0.; 3]; 4];
            let mut weights = [[0.; 3]; 4];
            for (k, corner) in corners[..n].iter().enumerate() {
                let (slot, pos) = match *corner {
                    Corner::Vertex(a) => {
                        weights[k][a] = 1.;
                        let i = face[a] as usize;
                        if !shading.is_empty() {
                            ns[k] = shading[3 * f + a];
                        }
                        (&mut remap[i], p[i])
                    }
                    Corner::Cut(a, b) => {
                        let (u, v) = (face[a], face[b]);
                        let (pt, t) = cut(p, &d, u, v);
                        weights[k][a] = 1. - t;
                        weights[k][b] = t;
                        if !shading.is_empty() {
                            let v = interpolate(shading[3 * f + a], shading[3 * f + b], t);
                            let len = v.iter().map(|x| x * x).sum::<f64>().sqrt();
                            ns[k] = if len > 0.0 { v.map(|x| x / len) } else { shading[3 * f + a] };
                        }
                        (cuts.entry(key(pt)).or_insert(u32::MAX), pt)
                    }
                };
                if *slot == u32::MAX {
                    let i = positions.len() / 3;
                    if i >= u32::MAX as usize {
                        return Err(invalid("trim exceeds the mesh vertex limit"));
                    }
                    *slot = i as u32;
                    positions.extend(pos);
                }
                verts[k] = *slot;
            }
            for i in 1..n - 1 {
                appearance.interpolate_face(f, [weights[0], weights[i], weights[i + 1]]).map_err(|e| invalid(&e))?;
                for j in [0, i, i + 1] {
                    indices.push(verts[j]);
                    if !shading.is_empty() {
                        normals.extend(ns[j]);
                    }
                }
            }
        }
        if indices.is_empty() {
            return Err(invalid("the selected side contains no triangles"));
        }
        if !removed {
            return Err(invalid("the plane does not trim this mesh on the selected side"));
        }
        self.positions = positions.into();
        self.triangles = indices.into();
        self.corner_normals = normals.into();
        self.appearance = appearance.finish();
        Ok(())
    }

    /// Shared endpoints and unique section segments. A patch lying in the plane contributes its perimeter.
    pub fn section_scan_plane(&self, plane: [f64; 4]) -> Result<(Vec<Point>, Vec<[usize; 2]>)> {
        let (p, faces) = self.scan_arrays()?;
        let d = distances(p, plane);
        let mut ids = HashMap::new();
        let mut pts = Vec::new();
        let mut edges = HashSet::new();
        let mut coplanar = HashMap::<[usize; 2], usize>::new();
        let mut segment = |a: Point, b: Point, coplanar_face: bool| {
            if key(a) == key(b) {
                return;
            }
            let mut pair = [a, b].map(|p| {
                *ids.entry(key(p)).or_insert_with(|| {
                    pts.push(p);
                    pts.len() - 1
                })
            });
            pair.sort_unstable();
            if coplanar_face {
                *coplanar.entry(pair).or_default() += 1;
            } else {
                edges.insert(pair);
            }
        };
        for &face in faces {
            let ds = face.map(|i| d[i as usize]);
            if ds.iter().all(|&x| x > 0.) || ds.iter().all(|&x| x < 0.) {
                continue;
            }
            let vertex = |i: usize| p[face[i] as usize];
            if ds == [0.; 3] {
                for i in 0..3 {
                    segment(vertex(i), vertex((i + 1) % 3), true);
                }
                continue;
            }
            let mut hits = [[0.; 3]; 3];
            let mut n = 0;
            for i in 0..3 {
                let j = (i + 1) % 3;
                if ds[i] == 0. {
                    hits[n] = vertex(i);
                    n += 1;
                } else if ds[j] != 0. && ds[i].is_sign_positive() != ds[j].is_sign_positive() {
                    hits[n] = cut(p, &d, face[i], face[j]).0;
                    n += 1;
                }
            }
            if n == 2 {
                segment(hits[0], hits[1], false);
            }
        }
        edges.extend(coplanar.into_iter().filter(|(_, n)| *n == 1).map(|(e, _)| e));
        if edges.is_empty() {
            return Err(invalid("the plane contains no section segments"));
        }
        let mut segments: Vec<_> = edges.into_iter().collect();
        segments.sort_unstable();
        Ok((pts, segments))
    }
}

#[cfg(test)]
mod tests {
    use crate::MeshData;
    fn square() -> MeshData {
        MeshData {
            appearance: None,
            name: "quad".into(),
            color: None,
            positions: vec![-1., -1., 0., 1., -1., 0., 1., 1., 0., -1., 1., 0.].into(),
            triangles: vec![0, 1, 2, 0, 2, 3].into(),
            corner_normals: [0., 0., 1.].repeat(6).into(),
        }
    }

    #[test]
    fn crossing_facets_are_split_with_shared_points_normals_and_winding() {
        for sign in [-1., 1.] {
            let mut m = square();
            m.trim_scan_plane([sign, 0., 0., 0.]).unwrap();
            assert_eq!(m.triangles.len(), 9);
            assert_eq!(m.positions.len(), 15);
            assert_eq!(m.corner_normals.len(), 27);
            let points = m.positions.as_chunks::<3>().0;
            assert!(points.iter().all(|p| sign * p[0] >= 0.));
            let area: f64 = m
                .triangles
                .as_chunks::<3>()
                .0
                .iter()
                .map(|f| {
                    let [a, b, c] = f.map(|i| points[i as usize]);
                    ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])) * 0.5
                })
                .sum();
            assert_eq!(area, 2.);
        }
        let mut m = square();
        assert!(m.trim_scan_plane([1., 0., 0., -2.]).is_err());
        assert_eq!(m.triangles, square().triangles);
    }
    #[test]
    fn plane_sections_join_at_shared_edges_and_ignore_coplanar_diagonals() {
        let m = square();
        let (_, edges) = m.section_scan_plane([0., 0., 1., 0.]).unwrap();
        assert_eq!(edges.len(), 4);
        let (pts, edges) = m.section_scan_plane([1., 0., 0., 0.]).unwrap();
        assert_eq!(pts.len(), 3);
        assert_eq!(edges.len(), 2);
        assert!(pts.iter().all(|p| p[0] == 0.));
        assert!(m.section_scan_plane([0., 1., 0., -4.]).is_err());
    }
}
