//! Editable scan remeshing with corner-chart preservation.
//!
//! Original implementation: independent link-condition edge collapses and conforming
//! bisection. Attribute discontinuities are pinned, following the discontinuity constraint
//! in Garland and Heckbert, "Simplifying Surfaces with Color and Texture", 1998.
//! Deviations: conservative midpoint contractions, no appearance quadric or atlas rebaking.
//! At most 24 reduction passes / 5 redistribution passes and 20 million input triangles.
use crate::{Result, SolidError, Vec3};

/// Optional per-corner data. Empty arrays mean the attribute is absent.
#[derive(Clone, Debug, Default)]
pub struct Attributes {
    /// Texture coordinates, independent at each triangle corner.
    pub uvs: Vec<[[f32; 2]; 3]>,
    /// Authored sRGB RGBA values.
    pub colors: Vec<[[u8; 4]; 3]>,
    /// Shading normals at triangle corners.
    pub normals: Vec<[[f64; 3]; 3]>,
    /// Material identity per triangle, or empty for one implicit material.
    pub materials: Vec<u32>,
}
/// Geometry and its corner data; indices are validated before any operation.
#[derive(Clone, Debug, Default)]
pub struct Scan {
    /// Positions in model units.
    pub positions: Vec<[f64; 3]>,
    /// Ordered triangle vertex indices.
    pub triangles: Vec<[u32; 3]>,
    /// Attributes whose indices follow triangles, never welded position indices.
    pub attributes: Attributes,
}
/// Requested editable mesh operation.
#[derive(Clone, Copy, Debug)]
pub enum Goal {
    /// Reduce toward this triangle count without crossing protected features.
    Reduce(usize),
    /// Redistribute edge lengths, using the initial mean when omitted.
    Remesh(Option<f64>),
}
/// Observable result; displacement is a conservative vertex ancestry bound, not Hausdorff error.
#[derive(Clone, Debug, Default)]
pub struct Report {
    /// Requested triangle target for reduction; zero for redistribution.
    pub requested_triangles: usize,
    /// Final triangle count.
    pub achieved_triangles: usize,
    /// Chosen edge length for redistribution.
    pub target_length: f64,
    /// Initial mean triangle edge length.
    pub mean_edge_before: f64,
    /// Final mean triangle edge length.
    pub mean_edge_after: f64,
    /// Initial mean normalized triangle quality.
    pub quality_before: f64,
    /// Final mean normalized triangle quality.
    pub quality_after: f64,
    /// Number of accepted contractions.
    pub collapsed_edges: usize,
    /// Number of conformingly bisected edges.
    pub split_edges: usize,
    /// Number of vertices pinned by attribute discontinuities on the last pass.
    pub appearance_pinned_vertices: usize,
    /// Refinement stopped at the output triangle budget.
    pub refinement_limited: bool,
    /// Protected topology or pass budget prevented the requested reduction.
    pub target_limited: bool,
    /// Maximum accumulated vertex motion bound in model units.
    pub max_vertex_motion_bound: f64,
}
fn invalid(why: &'static str) -> SolidError {
    SolidError::invalid_input("query::scan_remesh", why)
}
fn at<T: Copy>(values: &[T], i: usize) -> Result<T> {
    values
        .get(i)
        .copied()
        .ok_or_else(|| invalid("invalid mesh index"))
}
fn put<T>(values: &mut [T], i: usize, value: T) -> Result<()> {
    *values
        .get_mut(i)
        .ok_or_else(|| invalid("invalid mesh index"))? = value;
    Ok(())
}
fn point(s: &Scan, i: u32) -> Result<Vec3> {
    Ok(Vec3::from(at(&s.positions, i as usize)?))
}
fn normal(s: &Scan, f: [u32; 3]) -> Result<Vec3> {
    let [a, b, c] = f;
    Ok((point(s, b)? - point(s, a)?).cross(point(s, c)? - point(s, a)?))
}
fn key(a: u32, b: u32) -> u64 {
    (u64::from(a.min(b)) << 32) | u64::from(a.max(b))
}
fn ends(k: u64) -> [u32; 2] {
    [(k >> 32) as u32, k as u32]
}
fn vec_array(v: Vec3) -> [f64; 3] {
    [v.x, v.y, v.z]
}
fn reserve<T>(v: &mut Vec<T>, n: usize) -> Result<()> {
    v.try_reserve(n)
        .map_err(|_| invalid("scan workspace allocation failed"))
}
fn validate(s: &Scan) -> Result<()> {
    let n = s.triangles.len();
    if n == 0
        || n > 20_000_000
        || s.positions.len() > 20_000_000
        || s.positions
            .iter()
            .flatten()
            .any(|v| !v.is_finite() || v.abs() >= 1e9)
        || s.triangles
            .iter()
            .flatten()
            .any(|&v| v as usize >= s.positions.len())
    {
        return Err(invalid("invalid or oversized indexed scan"));
    }
    let a = &s.attributes;
    if [
        a.uvs.len(),
        a.colors.len(),
        a.normals.len(),
        a.materials.len(),
    ]
    .into_iter()
    .any(|l| l != 0 && l != n)
        || a.uvs.iter().flatten().flatten().any(|v| !v.is_finite())
        || a.normals.iter().flatten().flatten().any(|v| !v.is_finite())
    {
        return Err(invalid("invalid scan corner arrays"));
    }
    Ok(())
}
fn stats(s: &Scan) -> Result<(f64, f64)> {
    let (mut length, mut quality) = (0., 0.);
    for &[a, b, c] in &s.triangles {
        let (a, b, c) = (point(s, a)?, point(s, b)?, point(s, c)?);
        let sides = [
            (b - a).length_squared(),
            (c - b).length_squared(),
            (a - c).length_squared(),
        ];
        length += sides.iter().map(|v| v.sqrt()).sum::<f64>();
        let sum = sides.iter().sum::<f64>();
        if sum > 0. {
            quality += 2. * 3f64.sqrt() * (b - a).cross(c - a).length() / sum;
        }
    }
    Ok((
        length / (3 * s.triangles.len()) as f64,
        quality / s.triangles.len() as f64,
    ))
}
fn edges(s: &Scan) -> Result<Vec<u64>> {
    let mut keys = Vec::new();
    reserve(&mut keys, s.triangles.len() * 3)?;
    for &[a, b, c] in &s.triangles {
        keys.extend([key(a, b), key(b, c), key(c, a)]);
    }
    keys.sort_unstable();
    keys.dedup();
    Ok(keys)
}
struct Links {
    offsets: Vec<usize>,
    faces: Vec<u32>,
}
impl Links {
    fn new(s: &Scan) -> Result<Self> {
        let mut offsets = vec![0usize; s.positions.len() + 1];
        for &v in s.triangles.iter().flatten() {
            let slot = offsets
                .get_mut(v as usize + 1)
                .ok_or_else(|| invalid("invalid vertex"))?;
            *slot += 1;
        }
        let mut sum = 0;
        for n in &mut offsets {
            sum += *n;
            *n = sum;
        }
        let mut cursor = offsets.clone();
        let mut faces = vec![0; s.triangles.len() * 3];
        for (i, face) in s.triangles.iter().enumerate() {
            for &v in face {
                let c = cursor
                    .get_mut(v as usize)
                    .ok_or_else(|| invalid("invalid vertex"))?;
                put(&mut faces, *c, i as u32)?;
                *c += 1;
            }
        }
        Ok(Self { offsets, faces })
    }
    fn at(&self, v: u32) -> Result<&[u32]> {
        let a = at(&self.offsets, v as usize)?;
        let b = at(&self.offsets, v as usize + 1)?;
        self.faces
            .get(a..b)
            .ok_or_else(|| invalid("invalid adjacency"))
    }
}
#[derive(Clone, Copy, PartialEq)]
struct Chart {
    material: u32,
    uv: Option<[f32; 2]>,
    color: Option<[u8; 4]>,
}
fn chart(a: &Attributes, f: usize, k: usize) -> Result<Chart> {
    Ok(Chart {
        material: if a.materials.is_empty() {
            0
        } else {
            at(&a.materials, f)?
        },
        uv: if a.uvs.is_empty() {
            None
        } else {
            Some(at(&at(&a.uvs, f)?, k)?)
        },
        color: if a.colors.is_empty() {
            None
        } else {
            Some(at(&at(&a.colors, f)?, k)?)
        },
    })
}
fn chart_pins(s: &Scan) -> Result<(Vec<bool>, Vec<Option<Chart>>)> {
    let mut pinned = vec![false; s.positions.len()];
    let mut first = vec![None; s.positions.len()];
    for (f, tri) in s.triangles.iter().enumerate() {
        for (k, &v) in tri.iter().enumerate() {
            let c = chart(&s.attributes, f, k)?;
            let slot = first
                .get_mut(v as usize)
                .ok_or_else(|| invalid("invalid chart vertex"))?;
            if slot.is_some_and(|old| old != c) {
                put(&mut pinned, v as usize, true)?;
            } else {
                *slot = Some(c);
            }
        }
    }
    Ok((pinned, first))
}
fn pinned(v: u32, s: &Scan, links: &Links, cache: &mut [u8]) -> Result<bool> {
    let previous = at(cache, v as usize)?;
    if previous != 0 {
        return Ok(previous == 2);
    }
    let incident = links.at(v)?;
    let mut around = Vec::new();
    reserve(&mut around, incident.len() * 2)?;
    for &f in incident {
        let tri = at(&s.triangles, f as usize)?;
        let k = tri
            .iter()
            .position(|&x| x == v)
            .ok_or_else(|| invalid("broken adjacency"))?;
        around.push((at(&tri, (k + 1) % 3)?, true, f));
        around.push((at(&tri, (k + 2) % 3)?, false, f));
    }
    around.sort_unstable_by_key(|e| e.0);
    let mut pairs = Vec::new();
    let mut broken = false;
    for group in around.chunk_by(|a, b| a.0 == b.0) {
        let [a, b] = group else {
            broken = true;
            break;
        };
        if a.1 == b.1 {
            broken = true;
            break;
        }
        let na = normal(s, at(&s.triangles, a.2 as usize)?)?;
        let nb = normal(s, at(&s.triangles, b.2 as usize)?)?;
        if na.dot(nb)
            <= std::f64::consts::FRAC_1_SQRT_2 * (na.length_squared() * nb.length_squared()).sqrt()
        {
            broken = true;
            break;
        }
        pairs.push((a.2, b.2));
    }
    // Every incident face must belong to one fan, not a bow-tie pair of closed rings.
    if !broken && !incident.is_empty() {
        let seed = at(incident, 0)?;
        let mut seen = vec![seed];
        let mut cursor = 0;
        while cursor < seen.len() {
            let f = at(&seen, cursor)?;
            cursor += 1;
            for &(a, b) in &pairs {
                let next = if a == f {
                    Some(b)
                } else if b == f {
                    Some(a)
                } else {
                    None
                };
                if let Some(next) = next {
                    if !seen.contains(&next) {
                        seen.push(next);
                    }
                }
            }
        }
        broken = seen.len() != incident.len();
    }
    put(cache, v as usize, if broken { 2 } else { 1 })?;
    Ok(broken)
}
fn uv_area(uv: [[f32; 2]; 3]) -> f64 {
    let [[a, b], [c, d], [e, f]] = uv;
    (f64::from(c) - f64::from(a)) * (f64::from(f) - f64::from(b))
        - (f64::from(d) - f64::from(b)) * (f64::from(e) - f64::from(a))
}
fn blended(a: Chart, b: Chart) -> Chart {
    let uv = a
        .uv
        .zip(b.uv)
        .map(|(a, b)| {
            std::array::from_fn(|k| {
                (f64::from(*a.get(k).unwrap_or(&0.)) + f64::from(*b.get(k).unwrap_or(&0.))) * 0.5
            })
        })
        .map(|v: [f64; 2]| v.map(|x| x as f32));
    let color = a.color.zip(b.color).map(|(a, b)| {
        std::array::from_fn(|k| {
            ((u16::from(*a.get(k).unwrap_or(&0)) + u16::from(*b.get(k).unwrap_or(&0)) + 1) / 2)
                as u8
        })
    });
    Chart {
        material: a.material,
        uv,
        color,
    }
}
fn collapse(
    s: &mut Scan,
    keys: &[u64],
    threshold: Option<f64>,
    minimum: usize,
    motion: &mut [f64],
    report: &mut Report,
) -> Result<usize> {
    let links = Links::new(s)?;
    let (pins, charts) = chart_pins(s)?;
    report.appearance_pinned_vertices = pins.iter().filter(|&&v| v).count();
    let mut cache = vec![0; s.positions.len()];
    let mut touched = vec![false; s.triangles.len()];
    let mut dest: Vec<u32> = (0..s.positions.len() as u32).collect();
    let mut attributes = s.attributes.clone();
    let mut accepted = 0;
    let mut remaining = s.triangles.len();
    for &edge in keys {
        if remaining <= minimum.saturating_add(1) {
            break;
        }
        let [a, b] = ends(edge);
        let (pa, pb) = (point(s, a)?, point(s, b)?);
        let length = (pa - pb).length();
        if a == b
            || threshold.is_some_and(|t| length >= t)
            || at(&pins, a as usize)?
            || at(&pins, b as usize)?
        {
            continue;
        }
        let rings = [links.at(a)?, links.at(b)?];
        let mut conflict = false;
        for ring in rings {
            for &f in ring {
                if at(&touched, f as usize)? {
                    conflict = true;
                    break;
                }
            }
        }
        if conflict {
            continue;
        }
        if pinned(a, s, &links, &mut cache)? || pinned(b, s, &links, &mut cache)? {
            continue;
        }
        let ca = at(&charts, a as usize)?.ok_or_else(|| invalid("missing vertex chart"))?;
        let cb = at(&charts, b as usize)?.ok_or_else(|| invalid("missing vertex chart"))?;
        if ca.material != cb.material {
            continue;
        }
        let merged = blended(ca, cb);
        let mid = (pa + pb) * 0.5;
        let mut neighbours = [Vec::new(), Vec::new()];
        let mut ok = true;
        let mut shared = 0;
        for (side, ring) in rings.into_iter().enumerate() {
            for &f in ring {
                let tri = at(&s.triangles, f as usize)?;
                let v = if side == 0 { a } else { b };
                neighbours
                    .get_mut(side)
                    .ok_or_else(|| invalid("invalid side"))?
                    .extend(tri.into_iter().filter(|&k| k != v));
                if tri.contains(&a) && tri.contains(&b) {
                    if side == 0 {
                        shared += 1;
                    }
                    continue;
                }
                let mut pts = [Vec3::ZERO; 3];
                for (i, &k) in tri.iter().enumerate() {
                    put(&mut pts, i, if k == v { mid } else { point(s, k)? })?;
                }
                let [x, y, z] = pts;
                let old = normal(s, tri)?;
                let new = (y - x).cross(z - x);
                if old.dot(new) <= 0.2 * (old.length_squared() * new.length_squared()).sqrt() {
                    ok = false;
                    break;
                }
                if let Some(uv) = merged.uv {
                    let before = at(&attributes.uvs, f as usize)?;
                    let mut after = before;
                    for (j, &k) in tri.iter().enumerate() {
                        if k == v {
                            put(&mut after, j, uv)?;
                        }
                    }
                    let (old, new) = (uv_area(before), uv_area(after));
                    if old.abs() > 1e-20 && (old * new <= 0. || new.abs() < old.abs() * 0.1) {
                        ok = false;
                        break;
                    }
                }
            }
            if !ok {
                break;
            }
        }
        if !ok || shared != 2 {
            continue;
        }
        let [left, right] = &mut neighbours;
        left.sort_unstable();
        left.dedup();
        right.sort_unstable();
        right.dedup();
        if left
            .iter()
            .filter(|v| right.binary_search(v).is_ok())
            .count()
            != 2
            || (left.len() == 3 && right.len() == 3)
        {
            continue;
        }
        let bound =
            (at(motion, a as usize)? + length * 0.5).max(at(motion, b as usize)? + length * 0.5);
        put(motion, a as usize, bound)?;
        report.max_vertex_motion_bound = report.max_vertex_motion_bound.max(bound);
        put(&mut s.positions, a as usize, vec_array(mid))?;
        put(&mut dest, b as usize, a)?;
        remaining -= 2;
        accepted += 1;
        for ring in rings {
            for &f in ring {
                put(&mut touched, f as usize, true)?;
                let tri = at(&s.triangles, f as usize)?;
                for (j, &v) in tri.iter().enumerate() {
                    if v == a || v == b {
                        if let Some(uv) = merged.uv {
                            put(
                                attributes
                                    .uvs
                                    .get_mut(f as usize)
                                    .ok_or_else(|| invalid("invalid UV face"))?,
                                j,
                                uv,
                            )?;
                        }
                        if let Some(color) = merged.color {
                            put(
                                attributes
                                    .colors
                                    .get_mut(f as usize)
                                    .ok_or_else(|| invalid("invalid colour face"))?,
                                j,
                                color,
                            )?;
                        }
                    }
                }
            }
        }
    }
    if accepted > 0 {
        let mut faces = Vec::new();
        reserve(&mut faces, remaining)?;
        let mut out = Attributes::default();
        for (i, tri) in s.triangles.iter().enumerate() {
            let [a, b, c] = *tri;
            let face = [
                at(&dest, a as usize)?,
                at(&dest, b as usize)?,
                at(&dest, c as usize)?,
            ];
            let [a, b, c] = face;
            if a == b || b == c || a == c {
                continue;
            }
            faces.push(face);
            copy_attributes(&attributes, &mut out, i)?;
        }
        s.triangles = faces;
        s.attributes = out;
    }
    Ok(accepted)
}
fn copy_attributes(a: &Attributes, out: &mut Attributes, f: usize) -> Result<()> {
    if !a.uvs.is_empty() {
        out.uvs.push(at(&a.uvs, f)?);
    }
    if !a.colors.is_empty() {
        out.colors.push(at(&a.colors, f)?);
    }
    if !a.normals.is_empty() {
        out.normals.push(at(&a.normals, f)?);
    }
    if !a.materials.is_empty() {
        out.materials.push(at(&a.materials, f)?);
    }
    Ok(())
}
fn interpolated_attributes(
    a: &Attributes,
    out: &mut Attributes,
    f: usize,
    corners: [usize; 3],
) -> Result<()> {
    // Corner slots 0..3 are original vertices; 3..6 are edge midpoints.
    let weights = |k: usize| -> Result<[f64; 3]> {
        let mut w = [0.; 3];
        if k < 3 {
            put(&mut w, k, 1.)?;
        } else {
            let j = k - 3;
            put(&mut w, j, 0.5)?;
            put(&mut w, (j + 1) % 3, 0.5)?;
        }
        Ok(w)
    };
    if !a.uvs.is_empty() {
        let uv = at(&a.uvs, f)?;
        let mut result = [[0.; 2]; 3];
        for (i, &k) in corners.iter().enumerate() {
            let w = weights(k)?;
            let mut value = [0f64; 2];
            for (j, source) in uv.iter().enumerate() {
                let weight = at(&w, j)?;
                for (axis, &v) in source.iter().enumerate() {
                    let slot = value.get_mut(axis).ok_or_else(|| invalid("UV axis"))?;
                    *slot += f64::from(v) * weight;
                }
            }
            put(&mut result, i, value.map(|v| v as f32))?;
        }
        out.uvs.push(result);
    }
    if !a.colors.is_empty() {
        let color = at(&a.colors, f)?;
        let mut result = [[0; 4]; 3];
        for (i, &k) in corners.iter().enumerate() {
            let w = weights(k)?;
            let mut value = [0f64; 4];
            for (j, source) in color.iter().enumerate() {
                let weight = at(&w, j)?;
                for (axis, &v) in source.iter().enumerate() {
                    let slot = value.get_mut(axis).ok_or_else(|| invalid("colour axis"))?;
                    *slot += f64::from(v) * weight;
                }
            }
            put(
                &mut result,
                i,
                value.map(|v| v.round().clamp(0., 255.) as u8),
            )?;
        }
        out.colors.push(result);
    }
    if !a.normals.is_empty() {
        let normals = at(&a.normals, f)?;
        let mut result = [[0.; 3]; 3];
        for (i, &k) in corners.iter().enumerate() {
            let w = weights(k)?;
            let mut value = Vec3::ZERO;
            for (j, &source) in normals.iter().enumerate() {
                value = value + Vec3::from(source) * at(&w, j)?;
            }
            put(&mut result, i, vec_array(value.unitize_or_z()))?;
        }
        out.normals.push(result);
    }
    if !a.materials.is_empty() {
        out.materials.push(at(&a.materials, f)?);
    }
    Ok(())
}
fn split(
    s: &mut Scan,
    keys: Vec<u64>,
    target: f64,
    budget: usize,
    motion: &mut Vec<f64>,
) -> Result<(usize, bool)> {
    let mut candidates = Vec::new();
    for e in keys {
        let [a, b] = ends(e);
        if (point(s, a)? - point(s, b)?).length() > target * 4. / 3. {
            candidates.push(e);
        }
    }
    if candidates.is_empty() {
        return Ok((0, false));
    }
    let markers = |tri: [u32; 3], keys: &[u64]| -> Result<[Option<usize>; 3]> {
        let [a, b, c] = tri;
        Ok([key(a, b), key(b, c), key(c, a)].map(|k| keys.binary_search(&k).ok()))
    };
    let mut costs = vec![0usize; candidates.len()];
    for &f in &s.triangles {
        for i in markers(f, &candidates)?.into_iter().flatten() {
            let cost = costs
                .get_mut(i)
                .ok_or_else(|| invalid("invalid edge cost"))?;
            *cost += 1;
        }
    }
    let available = budget.saturating_sub(s.triangles.len());
    let required: usize = costs.iter().sum();
    let limited = required > available;
    if limited {
        let mut order = Vec::new();
        for (i, &k) in candidates.iter().enumerate() {
            let [a, b] = ends(k);
            order.push(((point(s, a)? - point(s, b)?).length_squared(), i, k));
        }
        order.sort_unstable_by(|a, b| b.0.total_cmp(&a.0).then(a.2.cmp(&b.2)));
        let mut remaining = available;
        let mut chosen = Vec::new();
        for (_, i, k) in order {
            let cost = at(&costs, i)?;
            if cost <= remaining {
                remaining -= cost;
                chosen.push(k);
            }
        }
        chosen.sort_unstable();
        candidates = chosen;
    }
    if candidates.is_empty() {
        return Ok((0, true));
    }
    if s.positions.len().saturating_add(candidates.len()) > 20_000_000 {
        return Ok((0, true));
    }
    let start = s.positions.len() as u32;
    reserve(&mut s.positions, candidates.len())?;
    reserve(motion, candidates.len())?;
    for &e in &candidates {
        let [a, b] = ends(e);
        s.positions
            .push(vec_array((point(s, a)? + point(s, b)?) * 0.5));
        motion.push(at(motion, a as usize)?.max(at(motion, b as usize)?));
    }
    let mut faces = Vec::new();
    reserve(&mut faces, s.triangles.len() + required.min(available))?;
    let mut attrs = Attributes::default();
    for (fi, &tri) in s.triangles.iter().enumerate() {
        let mids = markers(tri, &candidates)?;
        let [a, b, c] = tri;
        let mut corners = [a, b, c, 0, 0, 0];
        for (i, m) in mids.iter().enumerate() {
            if let Some(k) = m {
                put(&mut corners, i + 3, start + *k as u32)?;
            }
        }
        let mut emit = |f: [usize; 3]| -> Result<()> {
            let [a, b, c] = f;
            faces.push([at(&corners, a)?, at(&corners, b)?, at(&corners, c)?]);
            interpolated_attributes(&s.attributes, &mut attrs, fi, f)
        };
        match mids.iter().flatten().count() {
            0 => {
                faces.push(tri);
                copy_attributes(&s.attributes, &mut attrs, fi)?;
            }
            3 => {
                emit([0, 3, 5])?;
                emit([1, 4, 3])?;
                emit([2, 5, 4])?;
                emit([3, 4, 5])?;
            }
            1 => {
                let i = mids
                    .iter()
                    .position(Option::is_some)
                    .ok_or_else(|| invalid("missing split edge"))?;
                let j = (i + 1) % 3;
                let k = (i + 2) % 3;
                emit([i, i + 3, k])?;
                emit([i + 3, j, k])?;
            }
            _ => {
                let i = mids
                    .iter()
                    .position(Option::is_none)
                    .ok_or_else(|| invalid("missing unsplit edge"))?;
                let j = (i + 1) % 3;
                let k = (i + 2) % 3;
                emit([k, k + 3, j + 3])?;
                let len =
                    |a: usize, b: usize| -> Result<f64> {
                        Ok((point(s, at(&corners, a)?)? - point(s, at(&corners, b)?)?)
                            .length_squared())
                    };
                if len(i, j + 3)? <= len(j, k + 3)? {
                    emit([i, j, j + 3])?;
                    emit([i, j + 3, k + 3])?;
                } else {
                    emit([i, j, k + 3])?;
                    emit([j, j + 3, k + 3])?;
                }
            }
        }
    }
    s.triangles = faces;
    s.attributes = attrs;
    Ok((candidates.len(), limited))
}
fn compact(s: &mut Scan) -> Result<()> {
    let mut map = vec![u32::MAX; s.positions.len()];
    let mut positions = Vec::new();
    for v in s.triangles.iter_mut().flatten() {
        let slot = map
            .get_mut(*v as usize)
            .ok_or_else(|| invalid("invalid compaction index"))?;
        if *slot == u32::MAX {
            *slot = positions.len() as u32;
            positions.push(at(&s.positions, *v as usize)?);
        }
        *v = *slot;
    }
    s.positions = positions;
    Ok(())
}
/// Change triangulation while retaining material charts and interpolating only inside them.
///
/// The returned mesh owns new geometry/attribute arrays. Callers retain the input on error.
/// Boundary, non-manifold, inconsistent-winding, >45° crease and discontinuous attribute
/// vertices cannot contract. Midpoints split either side of a seam independently.
pub fn run(mut scan: Scan, goal: Goal) -> Result<(Scan, Report)> {
    validate(&scan)?;
    let (mean, quality) = stats(&scan)?;
    let mut report = Report {
        mean_edge_before: mean,
        quality_before: quality,
        ..Default::default()
    };
    let mut motion = vec![0.; scan.positions.len()];
    match goal {
        Goal::Reduce(target) => {
            if target == 0 || target >= scan.triangles.len() {
                return Err(invalid("reduction target must be smaller than the source"));
            }
            report.requested_triangles = target;
            for _ in 0..24 {
                if scan.triangles.len() <= target.saturating_add(1) {
                    break;
                }
                let mut candidates = Vec::new();
                for e in edges(&scan)? {
                    let [a, b] = ends(e);
                    candidates.push(((point(&scan, a)? - point(&scan, b)?).length_squared(), e));
                }
                candidates.sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
                let keys: Vec<_> = candidates.into_iter().map(|(_, e)| e).collect();
                let n = collapse(&mut scan, &keys, None, target, &mut motion, &mut report)?;
                report.collapsed_edges += n;
                if n == 0 {
                    break;
                }
            }
            report.target_limited = scan.triangles.len() > target.saturating_add(1);
        }
        Goal::Remesh(target) => {
            let target = target.unwrap_or(mean);
            if !target.is_finite() || target <= 0. {
                return Err(invalid("target edge length must be finite and positive"));
            }
            report.target_length = target;
            let budget = (4 * scan.triangles.len())
                .min(4_000_000)
                .max(scan.triangles.len())
                .max(4096);
            let passes = if scan.triangles.len() > 1_000_000 {
                2
            } else {
                5
            };
            for _ in 0..passes {
                let keys = edges(&scan)?;
                let n = collapse(
                    &mut scan,
                    &keys,
                    Some(0.8 * target),
                    1,
                    &mut motion,
                    &mut report,
                )?;
                let keys = if n > 0 { edges(&scan)? } else { keys };
                let (count, limited) = split(&mut scan, keys, target, budget, &mut motion)?;
                report.collapsed_edges += n;
                report.split_edges += count;
                report.refinement_limited |= limited;
                if n + count == 0 {
                    break;
                }
            }
        }
    }
    (report.mean_edge_after, report.quality_after) = stats(&scan)?;
    report.achieved_triangles = scan.triangles.len();
    compact(&mut scan)?;
    validate(&scan)?;
    Ok((scan, report))
}
