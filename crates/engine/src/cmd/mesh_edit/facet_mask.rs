use super::*;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct Args {
    body: String,
    /// Ordered inclusive ranges of original source triangle indices.
    ranges: Vec<[u32; 2]>,
    /// Session revision at which the mask was generated.
    revision: u64,
}
fn apply(s: &mut Session, p: &Value, id: &str, isolate: bool) -> Result<Value> {
    let args: Args = parse(p, id)?;
    if args.revision != s.revision {
        return Err(bad(id, "the design changed; draw the lasso again"));
    }
    edit(s, id, Some(&args.body), |m, _| m.retain_scan_facets(&args.ranges, isolate).map_err(|e| bad(id, e.to_string())))
}
pub(super) fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    apply(s, p, "mesh.delete_facets", false)
}
pub(super) fn isolate(s: &mut Session, p: &Value) -> Result<Value> {
    apply(s, p, "mesh.isolate_facets", true)
}

pub(super) fn append(ranges: &mut Vec<[u32; 2]>, id: u32) {
    if let Some(last) = ranges.last_mut()
        && last[1].checked_add(1) == Some(id)
    {
        last[1] = id
    } else {
        ranges.push([id, id])
    }
}

/// Scanline intervals give a winding-even-odd polygon test
/// whose cost per tested facet is independent of the number of lasso vertices.
pub(super) struct Screen {
    origin: [f64; 2],
    step: f64,
    width: usize,
    height: usize,
    rows: Vec<Vec<f64>>,
}
impl Screen {
    pub(super) fn new(poly: &[[f64; 2]]) -> Result<Self> {
        let id = "mesh.lasso";
        if !(3..=16384).contains(&poly.len()) || poly.iter().flatten().any(|p| !p.is_finite()) {
            return Err(bad(id, "draw a finite closed lasso"));
        }
        let mut lo = poly[0];
        let mut hi = lo;
        for p in poly {
            for k in 0..2 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        let extent = (hi[0] - lo[0]).max(hi[1] - lo[1]);
        if !extent.is_finite() || (hi[0] - lo[0]).min(hi[1] - lo[1]) < 1. {
            return Err(bad(id, "draw a larger lasso"));
        }
        let step = (extent / 2048.).max(0.5);
        let width = ((hi[0] - lo[0]) / step).ceil() as usize + 1;
        let height = ((hi[1] - lo[1]) / step).ceil() as usize + 1;
        let mut rows = vec![Vec::<f64>::new(); height];
        for (&a, &b) in poly.iter().zip(poly.iter().cycle().skip(1)) {
            let (a, b) = if a[1] <= b[1] { (a, b) } else { (b, a) };
            let start = ((a[1] - lo[1]) / step - 0.5).ceil().max(0.) as usize;
            let end = (((b[1] - lo[1]) / step - 0.5).ceil().max(0.) as usize).min(height);
            for (dy, row) in rows[start.min(height)..end].iter_mut().enumerate() {
                let y = lo[1] + (start as f64 + dy as f64 + 0.5) * step;
                row.push(a[0] + (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]));
            }
        }
        for row in &mut rows {
            row.sort_unstable_by(f64::total_cmp)
        }
        Ok(Self { origin: lo, step, width, height, rows })
    }
    fn xy(&self, p: [f64; 3]) -> [f64; 2] {
        [(p[0] - self.origin[0]) / self.step, (p[1] - self.origin[1]) / self.step]
    }
    pub(super) fn cell(&self, p: [f64; 3]) -> Option<usize> {
        let [x, y] = self.xy(p);
        if x < 0. || y < 0. || x >= self.width as f64 || y >= self.height as f64 {
            return None;
        }
        let iy = y as usize;
        (self.rows[iy].partition_point(|&v| v <= p[0]) % 2 == 1).then_some(iy * self.width + x as usize)
    }
}

#[derive(Clone, Copy)]
struct Candidate {
    facet: u32,
    center: [f64; 3],
    hidden: bool,
}

fn cross(a: [f64; 3], b: [f64; 3], p: [f64; 3]) -> f64 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}

/// Index sample points by screen x, then by y within narrow x stripes. A triangle considers
/// only centers actually inside its projected bounding box: on a zoomed-out scan many
/// independent facets can fit inside one pixel. A nearer sample elsewhere in that pixel
/// is not an occluder. Use f64 throughout, including the perspective reciprocal-depth plane.
fn remove_occluded(candidates: &mut [Candidate], projected: &[[f64; 3]], faces: &[[u32; 3]]) {
    if candidates.is_empty() {
        return;
    }
    candidates.sort_unstable_by(|a, b| a.center[0].total_cmp(&b.center[0]));
    const STRIPE: usize = 128;
    let mut xbounds = Vec::with_capacity(candidates.len().div_ceil(STRIPE));
    for stripe in candidates.chunks_mut(STRIPE) {
        xbounds.push((stripe[0].center[0], stripe[stripe.len() - 1].center[0]));
        stripe.sort_unstable_by(|a, b| a.center[1].total_cmp(&b.center[1]));
    }
    for (id, face) in faces.iter().enumerate() {
        let [a, b, c] = face.map(|v| projected[v as usize]);
        if !a[0].is_finite() || !b[0].is_finite() || !c[0].is_finite() {
            continue;
        }
        let area = cross(a, b, c);
        if area.abs() < 1e-13 {
            continue;
        }
        let (xmin, xmax) = (a[0].min(b[0]).min(c[0]), a[0].max(b[0]).max(c[0]));
        let (ymin, ymax) = (a[1].min(b[1]).min(c[1]), a[1].max(b[1]).max(c[1]));
        let first = xbounds.partition_point(|&(_, hi)| hi < xmin);
        let last = xbounds.partition_point(|&(lo, _)| lo <= xmax);
        if first == last {
            continue;
        }
        let end = (last * STRIPE).min(candidates.len());
        for stripe in candidates[first * STRIPE..end].chunks_mut(STRIPE) {
            let lo = stripe.partition_point(|s| s.center[1] < ymin);
            let hi = stripe.partition_point(|s| s.center[1] <= ymax);
            for sample in &mut stripe[lo..hi] {
                if sample.hidden || sample.facet == id as u32 || sample.center[0] < xmin || sample.center[0] > xmax {
                    continue;
                }
                let u = cross(b, c, sample.center) / area;
                let v = cross(c, a, sample.center) / area;
                let w = 1. - u - v;
                if u >= 0. && v >= 0. && w >= 0. {
                    let front = u * a[2] + v * b[2] + w * c[2];
                    let tol = 64. * f64::EPSILON * sample.center[2].abs().max(front.abs()).max(f64::MIN_POSITIVE);
                    if front + tol < sample.center[2] {
                        sample.hidden = true;
                    }
                }
            }
        }
    }
}

#[test]
fn tilted_subpixel_facets_select_the_entire_front_layer() {
    let n = 128usize;
    let mut projected = Vec::new();
    let mut faces = Vec::new();
    for layer in 0..2 {
        let base = projected.len() as u32;
        for y in 0..=n {
            for x in 0..=n {
                let (x, y) = (x as f64 * 0.045, y as f64 * 0.045);
                projected.push([x, y, 1. + 0.37 * x + 0.25 * y + layer as f64 * 3.]);
            }
        }
        for y in 0..n {
            for x in 0..n {
                let a = base + (y * (n + 1) + x) as u32;
                let b = a + 1;
                let c = b + n as u32 + 1;
                faces.extend([[a, b, c], [a, c, c - 1]]);
            }
        }
    }
    let mut candidates = faces
        .iter()
        .enumerate()
        .map(|(facet, t)| {
            let [a, b, c] = t.map(|id| projected[id as usize]);
            Candidate { facet: facet as u32, center: std::array::from_fn(|k| (a[k] + b[k] + c[k]) / 3.), hidden: false }
        })
        .collect::<Vec<_>>();
    remove_occluded(&mut candidates, &projected, &faces);
    let front = (2 * n * n) as u32;
    assert!(candidates.iter().all(|c| c.hidden == (c.facet >= front)));
}

impl Session {
    /// Source lasso ranges to full-resolution display primitive ids. The importer drops
    /// repeated-position corners but keeps collinear, distinct-position triangles.
    pub fn scan_facet_display_indices(&self, body: &str, ranges: &[[u32; 2]]) -> Result<Vec<u32>> {
        let src = source(self, "mesh.lasso", Some(body))?;
        let pts = src.mesh.positions.as_chunks::<3>().0;
        let faces = src.mesh.triangles.as_chunks::<3>().0;
        let mut last = None;
        for &[a, b] in ranges {
            if a > b || b as usize >= faces.len() || last.is_some_and(|v| a <= v) {
                return Err(bad("mesh.lasso", "invalid source facet range"));
            }
            last = Some(b);
        }
        let mut ranges = ranges.iter().peekable();
        let mut out = Vec::new();
        let mut display = 0;
        for (i, face) in faces.iter().enumerate() {
            let i = i as u32;
            if ranges.peek().is_some_and(|r| r[1] < i) {
                ranges.next();
            }
            let Some(range) = ranges.peek() else { break };
            let [a, b, c] = face.map(|v| pts[v as usize]);
            if a == b || a == c || b == c {
                continue;
            }
            if range[0] <= i {
                out.push(display);
            }
            display += 1;
        }
        Ok(out)
    }
    /// Evenly sampled world triangles of the resulting replace/add/subtract mask.
    pub fn scan_facet_preview(&self, body: &str, ranges: &[[u32; 2]]) -> Result<Vec<[Vec3; 3]>> {
        let src = source(self, "mesh.lasso", Some(body))?;
        let pts = src.mesh.positions.as_chunks::<3>().0;
        let faces = src.mesh.triangles.as_chunks::<3>().0;
        let mut count = 0usize;
        let mut last = None;
        for &[a, b] in ranges {
            if a > b || b as usize >= faces.len() || last.is_some_and(|v| a <= v) {
                return Err(bad("mesh.lasso", "invalid source facet range"));
            }
            count += b as usize - a as usize + 1;
            last = Some(b);
        }
        let n = count.min(4096);
        let mut output = Vec::with_capacity(n);
        let (mut range, mut offset) = (0, 0);
        for i in 0..n {
            let index = i * (count - 1) / n.saturating_sub(1).max(1);
            while index - offset > (ranges[range][1] - ranges[range][0]) as usize {
                offset += (ranges[range][1] - ranges[range][0]) as usize + 1;
                range += 1;
            }
            let id = ranges[range][0] as usize + index - offset;
            output.push(faces[id].map(|v| {
                let [x, y, z] = pts[v as usize];
                apply_point(&src.frame, Vec3::new(x, y, z))
            }));
        }
        Ok(output)
    }
    pub fn mesh_source_facet_ranges(&self, body: &str, indices: &[u32]) -> Result<Vec<[u32; 2]>> {
        let src = source(self, "mesh.lasso", Some(body))?;
        let pts = src.mesh.positions.as_chunks::<3>().0;
        let mut want = indices.to_vec();
        want.sort_unstable();
        want.dedup();
        let mut want = want.into_iter().peekable();
        let mut render = 0u32;
        let mut ranges = Vec::new();
        for (i, face) in src.mesh.triangles.as_chunks::<3>().0.iter().enumerate() {
            if want.peek().is_none() {
                break;
            }
            let [a, b, c] = face.map(|v| pts[v as usize]);
            if a == b || b == c || a == c {
                continue;
            }
            if want.next_if_eq(&render).is_some() {
                append(&mut ranges, i as u32)
            }
            render += 1;
        }
        if want.peek().is_some() {
            return Err(bad("mesh.lasso", "display facet outside current mesh"));
        }
        Ok(ranges)
    }
    /// Project returns logical screen x/y and a screen-affine depth increasing away from the eye.
    /// Use view depth for parallel cameras and negative reciprocal view depth for perspective.
    pub fn lasso_scan_facets(
        &self,
        body: &str,
        project: impl Fn(Vec3) -> Option<[f64; 3]>,
        polygon: &[[f64; 2]],
        visible_only: bool,
    ) -> Result<(Vec<[u32; 2]>, usize, Vec<[Vec3; 3]>)> {
        let src = source(self, "mesh.lasso", Some(body))?;
        let screen = Screen::new(polygon)?;
        let xyz = src.mesh.positions.as_chunks::<3>().0;
        let faces = src.mesh.triangles.as_chunks::<3>().0;
        let world = |p: [f64; 3]| apply_point(&src.frame, Vec3::new(p[0], p[1], p[2]));
        let projected: Vec<_> = xyz.iter().map(|&p| project(world(p)).filter(|a| a.iter().all(|x| x.is_finite())).unwrap_or([f64::NAN; 3])).collect();
        let mut candidates = Vec::new();
        for (i, face) in faces.iter().enumerate() {
            let tri = face.map(|v| projected[v as usize]);
            if tri.iter().any(|q| !q[0].is_finite()) {
                continue;
            }
            let [a, b, c] = tri;
            let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            // A valid facet seen exactly edge-on is still part of a through selection.
            if area.abs() < 1e-13 {
                use solidcraft_geom::Vec3Geom;
                let [a, b, c] = face.map(|v| world(xyz[v as usize]));
                if visible_only || (b - a).cross(c - a).len2() == 0.0 {
                    continue;
                }
            }
            let center = std::array::from_fn(|k| (a[k] + b[k] + c[k]) / 3.);
            if screen.cell(center).is_some() {
                candidates.push(Candidate { facet: i as u32, center, hidden: false });
            }
        }
        if visible_only {
            remove_occluded(&mut candidates, &projected, faces);
        }
        let mut selected: Vec<_> = candidates.into_iter().filter(|c| !c.hidden).map(|c| c.facet).collect();
        if visible_only {
            selected.sort_unstable();
        }
        let mut ranges = Vec::new();
        for &id in &selected {
            append(&mut ranges, id)
        }
        let sample = selected.len().min(4096);
        let overlay = (0..sample)
            .map(|i| {
                let id = selected[i * (selected.len() - 1) / sample.saturating_sub(1).max(1)];
                faces[id as usize].map(|v| world(xyz[v as usize]))
            })
            .collect();
        Ok((ranges, selected.len(), overlay))
    }
}
