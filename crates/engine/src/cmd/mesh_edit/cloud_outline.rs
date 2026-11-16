use super::*;
use solidcore_record::{Plane, Vec2};
use solidcraft_doc::PlaneRef;
use solidcraft_geom::{PlaneGeom, Vec3Geom};
use solidcraft_sketch::{CurveKind, Sketch};
use std::collections::{BTreeMap, BTreeSet};

const ID: &str = "pointcloud.section_profile";
type Cell = (i32, i32);
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct Args {
    cloud: cloud_copy::CloudKey,
    origin: [f64; 3],
    normal: [f64; 3],
    /// Full width of the slab in mm.
    thickness: f64,
    /// Side length of an occupied projected cell in mm.
    resolution: f64,
    name: Option<String>,
}
#[derive(serde::Serialize, JsonSchema)]
pub(super) struct Output {
    feature: u64,
    sketch: u64,
    name: String,
    outlines: usize,
    curves: usize,
    points: usize,
    resolution: f64,
    profile_report: String,
}
const DIR: [Cell; 4] = [(1, 0), (0, 1), (-1, 0), (0, -1)];
fn next(p: Cell, d: usize) -> Cell {
    (p.0 + DIR[d].0, p.1 + DIR[d].1)
}

/// Trace with the occupied cell on the left. At a diagonal contact turn left first:
/// two touching islands stay separate and hollow samples retain clockwise hole loops.
fn boundaries(cells: &BTreeSet<Cell>) -> Result<Vec<Vec<Cell>>> {
    let mut edges = BTreeMap::<Cell, u8>::new();
    for &(x, y) in cells {
        for (d, (neighbor, start)) in
            [((x, y - 1), (x, y)), ((x + 1, y), (x + 1, y)), ((x, y + 1), (x + 1, y + 1)), ((x - 1, y), (x, y + 1))].into_iter().enumerate()
        {
            if !cells.contains(&neighbor) {
                *edges.entry(start).or_default() |= 1 << d;
            }
        }
    }
    let mut loops = Vec::new();
    let mut segments = 0usize;
    while let Some((&start, &mask)) = edges.first_key_value() {
        let mut p = start;
        let mut d = mask.trailing_zeros() as usize;
        let mut corners = Vec::new();
        loop {
            let bits = edges.get_mut(&p).ok_or_else(|| bad(ID, "incomplete cell boundary"))?;
            *bits &= !(1 << d);
            if *bits == 0 {
                edges.remove(&p);
            }
            let q = next(p, d);
            if q == start {
                corners.push(q);
                break;
            }
            let options = edges.get(&q).copied().unwrap_or(0);
            let turn = [(d + 1) % 4, d, (d + 3) % 4].into_iter().find(|t| options & (1 << t) != 0).ok_or_else(|| bad(ID, "broken cell boundary"))?;
            if turn != d {
                corners.push(q);
            }
            p = q;
            d = turn;
        }
        if corners.len() >= 3 {
            let a = corners[corners.len() - 2];
            let b = corners[corners.len() - 1];
            let c = corners[0];
            if (a.0 == b.0 && b.0 == c.0) || (a.1 == b.1 && b.1 == c.1) {
                corners.pop();
            }
            segments += corners.len();
            if segments > 12000 {
                return Err(bad(ID, "too many outline corners; increase grid resolution or narrow the slab"));
            }
            loops.push(corners);
        }
    }
    Ok(loops)
}
#[test]
fn cloud_outline_preserves_holes_and_disconnected_parts() {
    let mut cells = BTreeSet::new();
    for x in 0..6 {
        for y in 0..5 {
            if x == 0 || x == 5 || y == 0 || y == 4 {
                cells.insert((x, y));
            }
        }
    }
    for x in 8..10 {
        for y in 0..3 {
            cells.insert((x, y));
        }
    }
    cells.insert((10, 3)); // A single point contact cannot merge the two islands.
    let paths = boundaries(&cells).unwrap();
    assert!(paths.iter().all(|p| p.len() == 4));
    let mut doubled_areas: Vec<i32> =
        paths.iter().map(|p| p.iter().zip(p.iter().cycle().skip(1)).map(|(a, b)| a.0 * b.1 - b.0 * a.1).sum()).collect();
    doubled_areas.sort_unstable();
    assert_eq!(doubled_areas, [-24, 2, 12, 60]);
}

pub(super) fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let a: Args = parse(p, ID)?;
    if !a.thickness.is_finite()
        || a.thickness <= 0.
        || !a.resolution.is_finite()
        || a.resolution <= 0.
        || !a.origin.iter().chain(&a.normal).all(|v| v.is_finite())
    {
        return Err(bad(ID, "enter a finite world plane and positive thickness and grid resolution"));
    }
    let key = match a.cloud {
        cloud_copy::CloudKey::Id(n) => n.to_string(),
        cloud_copy::CloudKey::Name(n) => n,
    };
    let c = cloud_copy::visible(s, &key, ID)?;
    let origin = Vec3::new(a.origin[0], a.origin[1], a.origin[2]);
    let n = Vec3::new(a.normal[0], a.normal[1], a.normal[2]).normalized().ok_or_else(|| bad(ID, "normal has zero length"))?;
    let plane = Plane::from_normal(origin, n).ok_or_else(|| bad(ID, "invalid section plane"))?;
    let mut cells = BTreeSet::new();
    let mut points = 0;
    {
        let shown = cloud_copy::shown(c);
        for i in 0..c.len() {
            let p = c.world_point(i).ok_or_else(|| bad(ID, "invalid source point"))?;
            if (p - origin).dot(n).abs() > a.thickness / 2. || !shown(i, p) {
                continue;
            }
            let q = plane.to_local(p);
            let (x, y) = ((q.x / a.resolution).floor(), (q.y / a.resolution).floor());
            if x.abs() > 1e9 || y.abs() > 1e9 {
                return Err(bad(ID, "point grid outside supported range; move the plane origin closer or enlarge cells"));
            }
            cells.insert((x as i32, y as i32));
            points += 1;
            if cells.len() > 1_000_000 {
                return Err(bad(ID, "over one million occupied cells; use larger cells or a thinner slab"));
            }
        }
    }
    if points < 3 {
        return Err(bad(ID, "the slab needs at least three visible points"));
    }
    let paths = boundaries(&cells)?;
    let mut sketch = Sketch::new();
    let mut curves = 0;
    {
        let mut batch = sketch.batch();
        for path in &paths {
            let mut vertices = Vec::with_capacity(path.len());
            for &(x, y) in path {
                vertices.push(batch.add_point(Vec2::new(x as f64 * a.resolution, y as f64 * a.resolution), None)?);
            }
            for (&a, &b) in vertices.iter().zip(vertices.iter().cycle().skip(1)) {
                batch.add_curve(CurveKind::Line { a, b }, None)?;
                curves += 1;
            }
        }
    }
    let mut kind = FeatureKind::Sketch { plane: PlaneRef::Custom { plane }, sketch };
    super::super::component::to_active_frame(s, &mut kind);
    let feature = s.doc_mut().add_feature(kind, Some(a.name.as_deref().unwrap_or("Cloud Outline")))?;
    let comp = s.active_component;
    let f = s.doc_mut().feature_mut(feature).ok_or_else(|| bad(ID, "new outline sketch is missing"))?;
    f.component = comp;
    let name = f.name.clone();
    s.refresh();
    if let Some(err) = s.model.result(feature).and_then(|r| r.error.as_ref()) {
        return Err(bad(ID, err));
    }
    let outlines = paths.len();
    Ok(json!(Output {
        feature,
        sketch: feature,
        name,
        outlines,
        curves,
        points,
        resolution: a.resolution,
        profile_report: format!("Created {outlines} occupied-area outlines from {points} slab points at {} mm grid resolution.", a.resolution)
    }))
}
