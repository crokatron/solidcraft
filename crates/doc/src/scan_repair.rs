//! Repair triangle incidence in welded coordinate space, retaining the source vertex layout.
use super::scan_cleanup::shared_vertices;
use crate::{DocError, MeshData, Result};
use solidcore_record::Vec3;
use solidcraft_geom::Vec3Geom;
use std::collections::{HashMap, HashSet};

fn invalid(s: &str) -> DocError {
    DocError::Invalid(s.into())
}
fn xyz(p: [f64; 3]) -> Vec3 {
    Vec3::new(p[0], p[1], p[2])
}
fn key(a: u32, b: u32) -> (u32, u32) {
    (a.min(b), a.max(b))
}
fn sides(t: [u32; 3]) -> [(u32, u32); 3] {
    [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])]
}
struct Edge {
    first: u32,
    second: Option<u32>,
    forward: bool,
    same: bool,
}
struct Topology<'a> {
    points: &'a [[f64; 3]],
    faces: &'a [[u32; 3]],
    weld: Vec<u32>,
    edges: HashMap<(u32, u32), Edge>,
}
impl<'a> Topology<'a> {
    fn new(m: &'a MeshData) -> Result<Self> {
        let (points, faces) = m.scan_arrays()?;
        if faces.len() >= u32::MAX as usize {
            return Err(invalid("too many triangles"));
        }
        let weld = shared_vertices(points);
        let mut edges = HashMap::<_, Edge>::new();
        for (i, t) in faces.iter().enumerate() {
            let [p, q, r] = t.map(|v| xyz(points[v as usize]));
            if (q - p).cross(r - p).len2() == 0. {
                return Err(invalid("clean degenerate triangles before repairing the scan"));
            }
            for (a, b) in sides(t.map(|v| weld[v as usize])) {
                match edges.entry(key(a, b)) {
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        entry.insert(Edge { first: i as u32, second: None, forward: a < b, same: false });
                    }
                    std::collections::hash_map::Entry::Occupied(mut entry) => {
                        let e = entry.get_mut();
                        if e.second.is_some() {
                            return Err(invalid("non-manifold edge has more than two incident faces"));
                        }
                        e.second = Some(i as u32);
                        e.same = e.forward == (a < b);
                    }
                }
            }
        }
        Ok(Self { points, faces, weld, edges })
    }
    fn position(&self, i: u32) -> Vec3 {
        xyz(self.points[i as usize])
    }
}
impl MeshData {
    pub fn orient_scan_normals(&mut self) -> Result<()> {
        let topology = Topology::new(self)?;
        let mut flip = vec![None; topology.faces.len()];
        let mut component = Vec::new();
        for start in 0..flip.len() {
            if flip[start].is_some() {
                continue;
            }
            component.clear();
            component.push(start);
            flip[start] = Some(false);
            let mut cursor = 0;
            let mut reversed = 0;
            while cursor < component.len() {
                let i = component[cursor];
                cursor += 1;
                let f = flip[i].unwrap_or(false);
                reversed += usize::from(f);
                for (a, b) in sides(topology.faces[i].map(|v| topology.weld[v as usize])) {
                    let Some(e) = topology.edges.get(&key(a, b)) else { return Err(invalid("missing scan edge")) };
                    let Some(second) = e.second else { continue };
                    let j = if e.first as usize == i { second } else { e.first } as usize;
                    let expected = f ^ e.same;
                    match flip[j] {
                        Some(actual) if actual != expected => return Err(invalid("the mesh contains a non-orientable surface")),
                        None => {
                            flip[j] = Some(expected);
                            component.push(j);
                        }
                        _ => {}
                    }
                }
            }
            // An open or nested shell has no absolute outside: minimize changed facets.
            if reversed > component.len() - reversed {
                for &i in &component {
                    flip[i] = flip[i].map(|v| !v)
                }
            }
        }
        let mut appearance = crate::mesh_edit_appearance::AppearanceBuilder::new(&self.appearance);
        let mut triangles = self.triangles.clone();
        let mut normals = self.corner_normals.clone();
        for (i, reversed) in flip.iter().enumerate() {
            let reversed = reversed == &Some(true);
            let weights = if reversed { [[1., 0., 0.], [0., 0., 1.], [0., 1., 0.]] } else { [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]] };
            appearance.interpolate_face(i, weights).map_err(|e| invalid(&e))?;
            if reversed {
                std::sync::Arc::make_mut(&mut triangles).swap(i * 3 + 1, i * 3 + 2);
                if !normals.is_empty() {
                    let ns = std::sync::Arc::make_mut(&mut normals);
                    for axis in 0..3 {
                        ns.swap(i * 9 + 3 + axis, i * 9 + 6 + axis)
                    }
                }
            }
            if !normals.is_empty() {
                let t = &triangles[i * 3..i * 3 + 3];
                let [a, b, c] = [t[0], t[1], t[2]].map(|j| topology.position(j));
                let n = (b - a).cross(c - a);
                for k in 0..3 {
                    let j = i * 9 + k * 3;
                    let v = Vec3::new(normals[j], normals[j + 1], normals[j + 2]);
                    if !v.is_finite() || v.len2() == 0. {
                        return Err(invalid("invalid shading normal"));
                    }
                    if v.dot(n) < 0. {
                        for x in &mut std::sync::Arc::make_mut(&mut normals)[j..j + 3] {
                            *x = -*x;
                        }
                    }
                }
            }
        }
        self.triangles = triangles;
        self.corner_normals = normals;
        self.appearance = appearance.finish();
        Ok(())
    }

    pub fn fill_scan_holes(&mut self, max_edges: u32) -> Result<()> {
        if !(3..=256).contains(&max_edges) {
            return Err(invalid("max_edges must be from 3 to 256"));
        }
        let top = Topology::new(self)?;
        let mut next = HashMap::new();
        let mut incoming = HashSet::new();
        for (&(a, b), e) in &top.edges {
            if e.same {
                return Err(invalid("orient inconsistent mesh normals before filling holes"));
            }
            if e.second.is_some() {
                continue;
            }
            // Reverse the existing border so the cap and old surface share opposite half edges.
            let (from, to) = if e.forward { (b, a) } else { (a, b) };
            if next.insert(from, (to, e.first)).is_some() || !incoming.insert(to) {
                return Err(invalid("branching boundary cannot be filled safely"));
            }
        }
        if next.is_empty() {
            return Ok(());
        }
        let mut ordered: Vec<_> = next.keys().copied().collect();
        ordered.sort_unstable_by(|a, b| top.position(*a).x.total_cmp(&top.position(*b).x).then(a.cmp(b)));
        let mut visited = HashSet::new();
        let mut caps = Vec::new();
        let mut nested = HashSet::new();
        for &start in &ordered {
            if visited.contains(&start) {
                continue;
            }
            let mut ring = Vec::new();
            let mut v = start;
            while visited.insert(v) {
                ring.push(v);
                v = next.get(&v).ok_or_else(|| invalid("boundary is not a closed loop"))?.0;
            }
            if v != start {
                return Err(invalid("invalid boundary cycle"));
            }
            if ring.len() < 3 || ring.len() > max_edges as usize {
                continue;
            }
            let Some(cap) = planar_cap(&ring, &top, &next, &ordered, &mut nested) else { continue };
            caps.push((ring, cap));
        }
        caps.retain(|(ring, _)| !ring.iter().any(|v| nested.contains(v)));
        if caps.is_empty() {
            return Err(invalid("no simple planar hole within the edge limit could be filled"));
        }
        let mut appearance = crate::mesh_edit_appearance::AppearanceBuilder::new(&self.appearance);
        for i in 0..self.triangles.len() / 3 {
            appearance.copy_face(i).map_err(|e| invalid(&e))?;
        }
        let mut triangles = self.triangles.to_vec();
        let mut normals = self.corner_normals.to_vec();
        for tri in caps.into_iter().flat_map(|(_, cap)| cap) {
            triangles.extend(tri);
            appearance.neutral_face().map_err(|e| invalid(&e))?;
            if !self.corner_normals.is_empty() {
                let [a, b, c] = tri.map(|v| top.position(v));
                let n = (b - a).cross(c - a).normalized().ok_or_else(|| invalid("degenerate cap"))?;
                for _ in 0..3 {
                    normals.extend([n.x, n.y, n.z]);
                }
            }
        }
        self.triangles = triangles.into();
        self.corner_normals = normals.into();
        self.appearance = appearance.finish();
        Ok(())
    }
}

type P = [f64; 2];
fn orient(a: P, b: P, c: P) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}
fn planar_cap(
    ring: &[u32],
    top: &Topology<'_>,
    boundary: &HashMap<u32, (u32, u32)>,
    ordered: &[u32],
    nested: &mut HashSet<u32>,
) -> Option<Vec<[u32; 3]>> {
    let origin = top.position(ring[0]);
    let pts: Vec<_> = ring.iter().map(|v| top.position(*v) - origin).collect();
    let scale = pts.iter().map(|p| p.len()).fold(0., f64::max);
    let eps = scale * 1e-7 + 1e-9;
    let area_eps = scale * eps * 0.01;
    let normal = (0..pts.len()).fold(Vec3::ZERO, |n, i| n + pts[i].cross(pts[(i + 1) % pts.len()])).normalized()?;
    if pts.iter().any(|p| p.dot(normal).abs() > eps) {
        return None;
    }
    let x = normal.any_perp();
    let y = normal.cross(x);
    let project = |p: Vec3| [p.dot(x), p.dot(y)];
    let xy: Vec<_> = pts.iter().map(|&p| project(p)).collect();
    let intersects = |a: P, b: P, c: P, d: P| {
        if (0..2).any(|k| a[k].max(b[k]) + eps < c[k].min(d[k]) || c[k].max(d[k]) + eps < a[k].min(b[k])) {
            return false;
        }
        let side = |v: f64| {
            if v > area_eps {
                1
            } else if v < -area_eps {
                -1
            } else {
                0
            }
        };
        side(orient(a, b, c)) * side(orient(a, b, d)) <= 0 && side(orient(c, d, a)) * side(orient(c, d, b)) <= 0
    };
    for i in 0..ring.len() {
        let k = (i + 1) % ring.len();
        // An exterior border of a planar sheet would cap the already-present sheet.
        let incident = top.faces[boundary.get(&ring[i])?.1 as usize];
        let third = incident.iter().map(|&v| top.weld[v as usize]).find(|v| *v != ring[i] && *v != ring[k])?;
        let p = top.position(third) - origin;
        if p.dot(normal).abs() <= eps && orient(xy[i], xy[k], project(p)) > area_eps {
            return None;
        }
        for j in i + 1..ring.len() {
            let l = (j + 1) % ring.len();
            if j == k || l == i {
                continue;
            }
            if intersects(xy[i], xy[k], xy[j], xy[l]) {
                return None;
            }
        }
    }
    // A cap containing another coplanar border (e.g. the end of a hollow tube) needs
    // a polygon with holes. Leave that boundary untouched rather than blocking the bore.
    let xmin = origin.x + pts.iter().map(|p| p.x).fold(f64::INFINITY, f64::min) - eps;
    let xmax = origin.x + pts.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max) + eps;
    let low = ordered.partition_point(|&v| top.position(v).x < xmin);
    let high = ordered.partition_point(|&v| top.position(v).x <= xmax);
    let own: HashSet<_> = ring.iter().copied().collect();
    let mut contains_ring = false;
    for &v in &ordered[low..high] {
        let q = top.position(v) - origin;
        if own.contains(&v) || q.dot(normal).abs() > eps {
            continue;
        }
        let q = project(q);
        if (0..xy.len()).fold(false, |inside, i| {
            let a = xy[i];
            let b = xy[(i + 1) % xy.len()];
            inside ^ ((a[1] > q[1]) != (b[1] > q[1]) && q[0] < (b[0] - a[0]) * (q[1] - a[1]) / (b[1] - a[1]) + a[0])
        }) {
            nested.insert(v);
            contains_ring = true;
        }
    }
    if contains_ring {
        return None;
    }
    // Ear clipping preserves concave contours; never fan-triangulate across a concavity.
    let mut order: Vec<_> = (0..ring.len()).collect();
    let mut out = Vec::new();
    while order.len() > 3 {
        let mut found = None;
        for i in 0..order.len() {
            let (a, b, c) = (order[(i + order.len() - 1) % order.len()], order[i], order[(i + 1) % order.len()]);
            if orient(xy[a], xy[b], xy[c]) <= area_eps {
                continue;
            }
            if top.edges.contains_key(&key(ring[a], ring[c])) {
                continue;
            }
            if order
                .iter()
                .any(|&j| j != a && j != b && j != c && [[a, b], [b, c], [c, a]].iter().all(|&[u, v]| orient(xy[u], xy[v], xy[j]) >= -area_eps))
            {
                continue;
            }
            out.push([ring[a], ring[b], ring[c]]);
            found = Some(i);
            break;
        }
        order.remove(found?);
    }
    let [a, b, c] = *order.as_slice().as_array::<3>()?;
    if orient(xy[a], xy[b], xy[c]) <= area_eps {
        return None;
    }
    out.push([ring[a], ring[b], ring[c]]);
    Some(out)
}
