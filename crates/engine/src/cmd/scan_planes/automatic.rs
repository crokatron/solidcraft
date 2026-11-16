use super::*;
use std::collections::HashMap;
const ID: &str = "mesh.auto_sections";
#[derive(Default, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum Axis {
    #[default]
    Auto,
    X,
    Y,
    Z,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct Args {
    body: Option<String>,
    #[serde(default)]
    axis: Axis,
    count: Option<usize>,
    offsets: Option<Vec<f64>>,
    tolerance: Option<f64>,
    name: Option<String>,
    /// Join the ends of each open, non-collinear contour with a straight sketch edge.
    #[serde(default)]
    close_open: bool,
}
#[derive(Serialize, JsonSchema)]
struct Profile {
    feature: u64,
    name: String,
    offset: f64,
    curves: usize,
    closing_edges: usize,
    closed_profiles: usize,
}
#[derive(Serialize, JsonSchema)]
pub(super) struct Output {
    axis: [f64; 3],
    profiles: Vec<Profile>,
    skipped_offsets: Vec<f64>,
    sections_report: String,
}

/// Stratified triangle samples weight the measured surface and avoid unused import vertices.
/// Robust end quantiles avoid putting the first and last profiles on isolated scanner debris.
fn stations(src: &mesh_edit::Source<'_>, a: &Args, count: usize) -> Result<(Vec3, Vec<f64>)> {
    let faces = src.mesh.triangles.as_chunks::<3>().0;
    let points = src.mesh.positions.as_chunks::<3>().0;
    let n = faces.len().min(16384);
    let mut samples = Vec::with_capacity(n);
    for i in 0..n {
        let f = faces[(i as u64 * faces.len() as u64 / n as u64) as usize];
        let p = f.map(|k| {
            let [x, y, z] = points[k as usize];
            apply_point(&src.frame, Vec3::new(x, y, z))
        });
        let weight = (p[1] - p[0]).cross(p[2] - p[0]).len();
        if weight > 0. && weight.is_finite() {
            samples.push(((p[0] + p[1] + p[2]) / 3., weight));
        }
    }
    if samples.is_empty() {
        return Err(bad(ID, "scan has no finite surface"));
    }
    let total = samples.iter().map(|s| s.1).sum::<f64>();
    let base = samples[0].0;
    let mean = base + samples.iter().fold(Vec3::ZERO, |sum, (p, w)| sum + (*p - base) * (*w / total));
    let axis = match a.axis {
        Axis::X => Vec3::X,
        Axis::Y => Vec3::Y,
        Axis::Z => Vec3::Z,
        Axis::Auto => {
            let mut cov = [[0.; 3]; 3];
            for &(p, w) in &samples {
                let d = p - mean;
                let v = [d.x, d.y, d.z];
                for i in 0..3 {
                    for j in 0..3 {
                        cov[i][j] += w / total * v[i] * v[j];
                    }
                }
            }
            let (_, axes) = mesh_edit::principal_axes(cov);
            let mut v = axes[2];
            let major = v.iter().copied().max_by(|a, b| a.abs().total_cmp(&b.abs())).unwrap_or(1.);
            if major < 0. {
                v = v.map(|x| -x);
            }
            Vec3::new(v[0], v[1], v[2])
        }
    };
    if let Some(values) = &a.offsets {
        if values.is_empty() || values.len() > 32 || values.iter().any(|v| !v.is_finite()) {
            return Err(bad(ID, "enter one to 32 finite station offsets"));
        }
        let mut values = values.clone();
        values.sort_by(f64::total_cmp);
        if values.windows(2).any(|v| (v[1] - v[0]).abs() < 1e-8) {
            return Err(bad(ID, "each station needs a distinct offset"));
        }
        return Ok((axis, values));
    }
    let mut projected: Vec<_> = samples.iter().map(|s| (axis.dot(s.0), s.1)).collect();
    projected.sort_by(|a, b| a.0.total_cmp(&b.0));
    // On a sparse CAD-derived mesh the section must span the full geometry rather than just its few facet centroids.
    let (lo, hi) = if faces.len() < 1024 {
        points.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &p| {
            let d = axis.dot(apply_point(&src.frame, p.into()));
            (lo.min(d), hi.max(d))
        })
    } else {
        let quantile = |q: f64| {
            let mut w = 0.;
            for &(d, weight) in &projected {
                w += weight;
                if w >= q * total {
                    return d;
                }
            }
            projected.last().map_or(0., |p| p.0)
        };
        (quantile(0.01), quantile(0.99))
    };
    if hi - lo < 1e-8 {
        return Err(bad(ID, "the scan has no depth along this axis; choose another direction"));
    }
    let mut offsets: Vec<_> = (0..count).map(|i| lo + (hi - lo) * (i as f64 + 0.5) / count as f64).collect();
    // Give every station its own band. Within it, favour changes in the transverse
    // section size, such as the shoulder of a handle. On a straight part use the midpoint.
    if samples.len() >= 512 && count > 1 {
        let bins = count * 8;
        let mut bands = vec![(0., Vec3::ZERO, 0., 0usize); bins];
        for &(p, w) in &samples {
            let t = (axis.dot(p) - lo) / (hi - lo);
            if !(0.0..1.0).contains(&t) {
                continue;
            }
            let q = p - mean;
            let radial = q - axis * q.dot(axis);
            let b = &mut bands[((t * bins as f64) as usize).min(bins - 1)];
            b.0 += w;
            b.1 = b.1 + radial * w;
            b.2 += w * radial.len2();
            b.3 += 1;
        }
        let radius = |i: usize| {
            let (w, sum, sq, n) = bands[i];
            (n >= 4 && w > 0.).then(|| (sq / w - (sum / w).len2()).max(0.).sqrt())
        };
        for (k, offset) in offsets.iter_mut().enumerate() {
            let mut threshold = 0.08;
            for j in k * 8 + 2..k * 8 + 6 {
                if let (Some(a), Some(b)) = (radius(j - 1), radius(j + 1)) {
                    let change = (a - b).abs() / a.max(b).max(1e-9);
                    if change > threshold {
                        threshold = change;
                        *offset = lo + (hi - lo) * (j as f64 + 0.5) / bins as f64;
                    }
                }
            }
        }
    }
    Ok((axis, offsets))
}

/// Trace the actual intersection graph. Stop at open ends and branches, without bridging scan gaps.
fn chains(n: usize, edges: &[[usize; 2]]) -> Vec<Vec<usize>> {
    let mut adj = vec![Vec::new(); n];
    for (i, &[a, b]) in edges.iter().enumerate() {
        adj[a].push((b, i));
        adj[b].push((a, i));
    }
    let mut seen = vec![false; edges.len()];
    let mut paths = Vec::new();
    for endpoints_first in [true, false] {
        for start in 0..n {
            if endpoints_first != (adj[start].len() != 2) {
                continue;
            }
            for &(mut to, mut edge) in &adj[start] {
                if seen[edge] {
                    continue;
                }
                let mut path = vec![start];
                loop {
                    seen[edge] = true;
                    path.push(to);
                    if to == start || adj[to].len() != 2 {
                        break;
                    }
                    let Some(&(next, e)) = adj[to].iter().find(|&&(_, i)| !seen[i]) else { break };
                    to = next;
                    edge = e;
                }
                paths.push(path);
            }
        }
    }
    paths
}
/// Iterative Douglas–Peucker. A closed ring is split at a distant vertex so the two retained arcs have a stable baseline.
fn simplify(path: &[usize], pts: &[Vec3], tol: f64) -> Vec<usize> {
    if path.len() <= 2 || tol == 0. {
        return path.to_vec();
    }
    let last = path.len() - 1;
    let mut keep = vec![false; path.len()];
    keep[0] = true;
    keep[last] = true;
    let mut work = vec![(0, last)];
    if path[0] == path[last] {
        let far = (1..last).max_by(|&a, &b| pts[path[a]].dist(pts[path[0]]).total_cmp(&pts[path[b]].dist(pts[path[0]]))).unwrap_or(1);
        keep[far] = true;
        work = vec![(0, far), (far, last)];
    }
    while let Some((lo, hi)) = work.pop() {
        let mut best = tol;
        let mut split = None;
        for i in lo + 1..hi {
            let d = pts[path[i]].dist_to_segment(pts[path[lo]], pts[path[hi]]);
            if d > best {
                best = d;
                split = Some(i)
            }
        }
        if let Some(i) = split {
            keep[i] = true;
            work.extend([(lo, i), (i, hi)]);
        }
    }
    let result: Vec<_> = path.iter().zip(keep).filter_map(|(&p, yes)| yes.then_some(p)).collect();
    if path[0] == path[last] && result.len() < 4 {
        return path.to_vec();
    }
    result
}
fn make_sketch(src: &mesh_edit::Source<'_>, axis: Vec3, offset: f64, tol: f64, close_open: bool) -> Result<Option<(FeatureKind, usize, usize)>> {
    let (plane, _) = cutting_plane(Principal::XY, Some([axis.x, axis.y, axis.z]), offset, ID)?;
    let (pts, edges) = match src.mesh.section_scan_plane(equation(&src.frame, axis, offset)) {
        Ok(v) => v,
        Err(e) if e.to_string().contains("no section segments") => return Ok(None),
        Err(e) => return Err(bad(ID, e.to_string())),
    };
    let pts: Vec<_> = pts.into_iter().map(|p| apply_point(&src.frame, p.into())).collect();
    let mut sketch = Sketch::new();
    let mut ids = HashMap::new();
    let mut count = 0;
    let mut batch = sketch.batch();
    let mut closing_edges = 0;
    for mut path in chains(pts.len(), &edges) {
        if path.len() > 3 && path.first() == path.last() {
            path.pop();
            let n = path.len();
            // Start a closed contour at a corner. Starting on the middle of a
            // straight side would leave a redundant sketch vertex at the seam.
            let bend = |i: usize| {
                let at = pts[path[i]];
                let a = (at - pts[path[(i + n - 1) % n]]).normalized();
                let b = (pts[path[(i + 1) % n]] - at).normalized();
                a.zip(b).map_or(0., |(a, b)| 1. - a.dot(b))
            };
            let corner = (0..n).max_by(|&a, &b| bend(a).total_cmp(&bend(b))).unwrap_or(0);
            path.rotate_left(corner);
            path.push(path[0]);
        }
        let mut path = simplify(&path, &pts, tol);
        if close_open && path.len() >= 3 && path.first() != path.last() {
            let first = pts[path[0]];
            let last = pts[path[path.len() - 1]];
            if path.iter().any(|&i| pts[i].dist_to_segment(first, last) > tol.max(1e-8)) {
                path.push(path[0]);
                closing_edges += 1;
            }
        }
        for pair in path.windows(2) {
            let mut ends = [0; 2];
            for (i, &j) in pair.iter().enumerate() {
                let id = match ids.get(&j) {
                    Some(&id) => id,
                    None => {
                        let id = batch.add_point(plane.to_local(pts[j]), None)?;
                        ids.insert(j, id);
                        id
                    }
                };
                ends[i] = id;
            }
            batch.add_curve(CurveKind::Line { a: ends[0], b: ends[1] }, None)?;
            count += 1;
        }
    }
    Ok(Some((FeatureKind::Sketch { plane: PlaneRef::Custom { plane }, sketch }, count, closing_edges)))
}
pub(super) fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let args: Args = serde_json::from_value(p.clone()).map_err(|e| bad(ID, e.to_string()))?;
    let count = args.count.unwrap_or(5);
    let tol = args.tolerance.unwrap_or(0.05);
    if !(1..=32).contains(&count) || !tol.is_finite() || tol < 0. {
        return Err(bad(ID, "choose 1 to 32 sections and a nonnegative tolerance"));
    }
    let src = mesh_edit::source(s, ID, args.body.as_deref())?;
    let (axis, offsets) = stations(&src, &args, count)?;
    let mut prepared = Vec::new();
    let mut skipped = Vec::new();
    for offset in offsets {
        if let Some((kind, n, closing_edges)) = make_sketch(&src, axis, offset, tol, args.close_open)? {
            prepared.push((offset, kind, n, closing_edges));
        } else {
            skipped.push(offset);
        }
    }
    if prepared.is_empty() {
        return Err(bad(ID, "none of the stations intersect the scan"));
    }
    let mut profiles = Vec::new();
    let comp = s.active_component;
    let base = args.name.as_deref().map(str::trim).filter(|v| !v.is_empty()).unwrap_or("Scan Profile");
    let mut number = 1usize;
    for (offset, mut kind, curves, closing_edges) in prepared {
        super::super::component::to_active_frame(s, &mut kind);
        let name = loop {
            let candidate = format!("{base} {number}");
            number += 1;
            if !s.doc.features.iter().any(|f| f.name == candidate) {
                break candidate;
            }
        };
        let feature = s.doc_mut().add_feature(kind, Some(&name))?;
        let f = s.doc_mut().feature_mut(feature).ok_or_else(|| bad(ID, "new profile is missing"))?;
        f.component = comp;
        profiles.push(Profile { feature, name: f.name.clone(), offset, curves, closing_edges, closed_profiles: 0 });
    }
    s.refresh();
    let mut closed = 0;
    for row in &mut profiles {
        if let Some(e) = s.model.result(row.feature).and_then(|r| r.error.as_ref()) {
            return Err(bad(ID, e));
        }
        if let Some(sk) = s.model.state().sketch(row.feature) {
            row.closed_profiles = sk.profiles.len();
            closed += sk.profiles.len();
        }
    }
    let added: usize = profiles.iter().map(|p| p.closing_edges).sum();
    let sections_report = format!(
        "Created {} editable section sketches with {closed} closed profiles. Added {added} straight closing edges. Skipped {} empty stations.",
        profiles.len(),
        skipped.len()
    );
    Ok(json!(Output { axis: [axis.x, axis.y, axis.z], profiles, skipped_offsets: skipped, sections_report }))
}
