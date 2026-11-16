use super::*;
use solidcraft_doc::point_cloud::{ColorBy, Colors, Complex, Keep, MAX_POINTS, On, PointCloud, Points, Source as Property};
use std::collections::HashSet;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct Whole {
    body: String,
    name: Option<String>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct Patch {
    body: String,
    name: Option<String>,
    ranges: Vec<[u32; 2]>,
    revision: u64,
}
#[derive(Deserialize, JsonSchema)]
#[serde(untagged)]
pub(super) enum CloudKey {
    Id(u64),
    Name(String),
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct Snippet {
    cloud: CloudKey,
    name: Option<String>,
    ranges: Vec<[u32; 2]>,
    revision: u64,
}
#[derive(serde::Serialize, JsonSchema)]
pub(super) struct Copied {
    cloud: u64,
    name: String,
    points: usize,
    copy_report: String,
}

fn checked_ranges(ranges: &[[u32; 2]], len: usize, cmd: &str) -> Result<()> {
    if ranges.is_empty() {
        return Err(bad(cmd, "select at least one facet or point"));
    }
    let mut last = None;
    for &[a, b] in ranges {
        if a > b || b as usize >= len || last.is_some_and(|n| a <= n) {
            return Err(bad(cmd, "selection ranges must be ordered, disjoint and in bounds"));
        }
        last = Some(b);
    }
    Ok(())
}
fn current(s: &Session, rev: u64, id: &str) -> Result<()> {
    if s.revision == rev { Ok(()) } else { Err(bad(id, "the design changed; draw the selection again")) }
}

fn store(s: &mut Session, mut c: PointCloud, requested: Option<String>, base: String, cmd: &str) -> Result<Value> {
    if c.is_empty() {
        return Err(bad(cmd, "selection contains no visible points"));
    }
    let base = requested.as_deref().map(str::trim).filter(|v| !v.is_empty()).unwrap_or(&base);
    let taken = |name: &str| s.doc.point_clouds.iter().any(|c| c.name == name);
    c.name = if !taken(base) {
        base.into()
    } else {
        (2..=s.doc.point_clouds.len() + 2)
            .map(|i| format!("{base} ({i})"))
            .find(|n| !taken(n))
            .ok_or_else(|| bad(cmd, "could not allocate a unique cloud name"))?
    };
    c.source_points = c.len() as u64;
    c.check()?;
    let d = s.doc_mut();
    c.id = d.allocate_id()?;
    let result =
        json!(Copied { cloud: c.id, name: c.name.clone(), points: c.len(), copy_report: format!("Copied {} points to {}.", c.len(), c.name) });
    d.point_clouds.push(c);
    Ok(result)
}

fn mesh_cloud(s: &mut Session, cmd: &str, body: &str, name: Option<String>, ranges: Option<&[[u32; 2]]>) -> Result<Value> {
    let src = source(s, cmd, Some(body))?;
    if src.mesh.appearance.as_ref().is_some_and(|a| a.materials.iter().any(|m| m.texture.is_some())) {
        return Err(bad(
            cmd,
            "copying a photographic mesh to a coloured point cloud requires texture baking and is not supported yet; the original scan was not changed",
        ));
    }
    let positions = src.mesh.positions.as_chunks::<3>().0;
    let faces = src.mesh.triangles.as_chunks::<3>().0;
    let mut selected = vec![false; positions.len()];
    let whole;
    let ranges = if let Some(ranges) = ranges {
        checked_ranges(ranges, faces.len(), cmd)?;
        ranges
    } else {
        if faces.is_empty() {
            return Err(bad(cmd, "source scan is empty"));
        }
        whole = [[0, faces.len() as u32 - 1]];
        &whole
    };
    for &[a, b] in ranges {
        for f in &faces[a as usize..=b as usize] {
            for &vertex in f {
                *selected.get_mut(vertex as usize).ok_or_else(|| bad(cmd, "invalid source mesh vertex"))? = true;
            }
        }
    }
    // A binary STL can repeat every corner. Deduplicate exact coordinates before f32 storage.
    let mut distinct = HashSet::new();
    let mut out = Vec::new();
    let mut origin = None;
    let mut colors = Vec::<u8>::new();
    if let Some(appearance) = &src.mesh.appearance {
        appearance.validate(faces.len()).map_err(|e| bad(cmd, e))?;
        let mut colored_distinct = HashSet::new();
        for &[a, b] in ranges {
            for i in a as usize..=b as usize {
                let material = appearance.triangle_materials.get(i).copied().unwrap_or(0) as usize;
                let tint = appearance.materials.get(material).map(|m| m.base_color).unwrap_or([1.; 4]);
                for j in 0..3 {
                    let p = positions[faces[i][j] as usize];
                    let rgb = appearance.corner_colors.get(i).map(|c| c[j]).unwrap_or([255; 4]);
                    let color: [u8; 3] = std::array::from_fn(|k| (f32::from(rgb[k]) * tint[k]).round().clamp(0., 255.) as u8);
                    let key = p.map(|v| if v == 0. { 0 } else { v.to_bits() });
                    if !colored_distinct.insert((key, color)) {
                        continue;
                    }
                    if colored_distinct.len() > MAX_POINTS {
                        return Err(bad(cmd, "selection exceeds the point cloud limit"));
                    }
                    let center = *origin.get_or_insert(p);
                    out.extend((0..3).map(|k| (p[k] - center[k]) as f32));
                    colors.extend(color);
                }
            }
        }
    } else {
        for (&p, keep) in positions.iter().zip(selected) {
            if !keep || !distinct.insert(p.map(|v| if v == 0. { 0 } else { v.to_bits() })) {
                continue;
            }
            if distinct.len() > MAX_POINTS {
                return Err(bad(cmd, "selection exceeds the point cloud limit"));
            }
            let center = *origin.get_or_insert(p);
            out.extend((0..3).map(|k| (p[k] - center[k]) as f32));
        }
    }
    let [x, y, z] = origin.ok_or_else(|| bad(cmd, "no source vertices"))?;
    let has_colors = !colors.is_empty();
    let c = PointCloud {
        id: 0,
        name: String::new(),
        file: String::new(),
        origin: Vec3::new(x, y, z),
        points: Points::new(out.into()),
        colors: Colors(colors.into()),
        source_points: 0,
        units: "mm".into(),
        transform: src.frame,
        point_size: 2.,
        color_by: if has_colors { ColorBy::File } else { ColorBy::Single },
        color: src.mesh.color,
        height_range: None,
        visible: true,
        complex: Complex::default(),
        color_property: None,
        filters: Vec::new(),
        source: String::new(),
        max_points: None,
    };
    store(s, c, name, format!("{body} {}", if cmd == "mesh.copy_point_cloud" { "points" } else { "patch" }), cmd)
}
#[test]
fn independent_embedded_clouds_keep_their_positions_filters_and_undo() {
    let mut d = solidcraft_doc::Document::default();
    let mesh = MeshData {
        appearance: None,
        name: "Scan".into(),
        positions: vec![0., 0., 0., 2., 0., 0., 2., 2., 0., 0., 0., 0., 2., 2., 0., 0., 2., 0., 9., 9., 9.].into(),
        triangles: vec![0, 1, 2, 3, 4, 5].into(),
        corner_normals: Default::default(),
        color: None,
    };
    let id = d.add_feature(FeatureKind::MeshImport { file: "in.stl".into(), meshes: vec![mesh] }, None).unwrap();
    d.feature_mut(id).unwrap().body_names = vec!["Scan".into()];
    let mut m = solidcraft_doc::IDENTITY;
    m[0] = [0., 1., 0., 0.];
    m[1] = [-1., 0., 0., 0.];
    m[3] = [20., 30., 40., 1.];
    d.body_offsets.insert("Scan".into(), m);
    let before = d.to_json();
    let mut s = Session::new(d);
    s.param_check = crate::schema::ParamCheck::Strict;
    let r = s.execute("mesh.copy_point_cloud", &json!({"body":"Scan","name":"Reference"})).unwrap();
    assert_eq!(r["points"], 4);
    let cloud = &s.doc.point_clouds[0];
    let p = cloud.world_point(1).unwrap();
    assert_eq!([p.x, p.y, p.z], [20., 32., 40.]);
    let rev = s.revision;
    let patch = s.execute("mesh.copy_facet_cloud", &json!({"body":"Scan","ranges":[[1,1]],"revision":rev,"name":"Corner"})).unwrap();
    assert_eq!(patch["points"], 3);
    assert!(s.execute("mesh.copy_facet_cloud", &json!({"body":"Scan","ranges":[[1,1]],"revision":rev})).is_err());
    s.execute("pointcloud.edit", &json!({"cloud":r["cloud"],"filters":[{"property":"x","range":[19.,21.]}]})).unwrap();
    let poly = [[15., 25.], [25., 25.], [25., 35.], [15., 35.]];
    let (ranges, count, preview) = s.lasso_cloud_points("Reference", |p| Some([p.x, p.y, p.z]), &poly).unwrap();
    assert_eq!(count, 2);
    assert_eq!(preview.len(), 2);
    assert_eq!(ranges, [[0, 1]]);
    let rev = s.revision;
    let snippet = s.execute("pointcloud.copy_snippet", &json!({"cloud":r["cloud"],"ranges":ranges,"revision":rev,"name":"Rail"})).unwrap();
    assert_eq!(snippet["points"], 2);
    s.execute("pointcloud.edit", &json!({"cloud":"Reference","visible":false})).unwrap();
    assert!(!s.doc.point_clouds[0].visible && s.doc.point_clouds[1].visible && s.doc.point_clouds[2].visible);
    assert!(s.lasso_cloud_points("Reference", |p| Some([p.x, p.y, p.z]), &poly).is_err());
    let cold = solidcraft_doc::Document::from_json(&s.doc.to_json()).unwrap();
    assert_eq!(cold.point_clouds, s.doc.point_clouds);
    for _ in 0..5 {
        s.execute("edit.undo", &json!({})).unwrap();
    }
    assert_eq!(s.doc.to_json(), before);
}

pub(super) fn all(s: &mut Session, p: &Value) -> Result<Value> {
    let id = "mesh.copy_point_cloud";
    let a: Whole = parse(p, id)?;
    mesh_cloud(s, id, &a.body, a.name, None)
}
pub(super) fn facets(s: &mut Session, p: &Value) -> Result<Value> {
    let id = "mesh.copy_facet_cloud";
    let a: Patch = parse(p, id)?;
    current(s, a.revision, id)?;
    mesh_cloud(s, id, &a.body, a.name, Some(&a.ranges))
}

pub(super) fn visible<'a>(s: &'a Session, key: &str, id: &str) -> Result<&'a PointCloud> {
    crate::view::shown_point_clouds(s).find(|c| c.id.to_string() == key || c.name == key).ok_or_else(|| bad(id, "choose a visible point cloud"))
}
pub(crate) fn shown(c: &PointCloud) -> impl Fn(usize, Vec3) -> bool + '_ {
    let filters: Vec<_> = c.filters.iter().map(|f| (c.source(&f.property, On::Point), &f.keep)).collect();
    move |i, p| {
        filters.iter().all(|(src, keep)| {
            let value = match src {
                Some(Property::Coordinate(k)) => [p.x, p.y, p.z][*k],
                Some(Property::Element(v) | Property::Corners(v)) => match v.get(i) {
                    Some(&value) => f64::from(value),
                    None => return false,
                },
                // The viewport applies a cell filter only when that cell has the property.
                None => return true,
            };
            match keep {
                Keep::Range { range: [a, b] } => value >= *a && value <= *b,
                Keep::Values { values } => value.is_finite() && values.contains(&(value.round() as i32)),
            }
        })
    }
}
impl Session {
    pub fn lasso_cloud_points(
        &self,
        cloud: &str,
        project: impl Fn(Vec3) -> Option<[f64; 3]>,
        polygon: &[[f64; 2]],
    ) -> Result<(Vec<[u32; 2]>, usize, Vec<Vec3>)> {
        let c = visible(self, cloud, "pointcloud.copy_snippet")?;
        let screen = super::facet_mask::Screen::new(polygon)?;
        let shown = shown(c);
        let mut ranges = Vec::new();
        let mut count = 0usize;
        for i in 0..c.len() {
            let p = c.world_point(i).ok_or_else(|| bad("pointcloud.copy_snippet", "invalid source point"))?;
            if shown(i, p) && project(p).is_some_and(|q| q.iter().all(|v| v.is_finite()) && screen.cell(q).is_some()) {
                super::facet_mask::append(&mut ranges, i as u32);
                count += 1;
            }
        }
        let preview = self.cloud_point_preview(cloud, &ranges)?;
        Ok((ranges, count, preview))
    }

    pub fn cloud_point_preview(&self, cloud: &str, ranges: &[[u32; 2]]) -> Result<Vec<Vec3>> {
        let c = visible(self, cloud, "pointcloud.copy_snippet")?;
        if ranges.is_empty() {
            return Ok(Vec::new());
        }
        checked_ranges(ranges, c.len(), "pointcloud.copy_snippet")?;
        let n: usize = ranges.iter().map(|r| (r[1] - r[0] + 1) as usize).sum();
        let samples = n.min(4096);
        let mut offset = 0;
        let mut range = 0;
        (0..samples)
            .map(|i| {
                let at = i * (n - 1) / samples.saturating_sub(1).max(1);
                while at - offset > (ranges[range][1] - ranges[range][0]) as usize {
                    offset += (ranges[range][1] - ranges[range][0]) as usize + 1;
                    range += 1;
                }
                c.world_point(ranges[range][0] as usize + at - offset).ok_or_else(|| bad("pointcloud.copy_snippet", "invalid preview point"))
            })
            .collect()
    }
}
pub(super) fn snippet(s: &mut Session, p: &Value) -> Result<Value> {
    let id = "pointcloud.copy_snippet";
    let a: Snippet = parse(p, id)?;
    current(s, a.revision, id)?;
    let key = match a.cloud {
        CloudKey::Id(i) => i.to_string(),
        CloudKey::Name(n) => n,
    };
    let src = visible(s, &key, id)?;
    checked_ranges(&a.ranges, src.len(), id)?;
    let mut points = Vec::new();
    let mut colors = Vec::new();
    let mut mask = vec![false; src.len()];
    let keep = shown(src);
    for [lo, hi] in a.ranges {
        for i in lo as usize..=hi as usize {
            let p = src.world_point(i).ok_or_else(|| bad(id, "invalid source point"))?;
            if keep(i, p) {
                mask[i] = true;
                points.extend(src.points.get(i).ok_or_else(|| bad(id, "invalid source point"))?);
                if let Some(rgb) = src.colors.get(i) {
                    colors.extend(rgb);
                }
            }
        }
    }
    drop(keep);
    let base = format!("{} snippet", src.name);
    let mut c = src.clone();
    c.complex = src.complex.induced(&mask).0;
    c.points = Points::new(points.into());
    c.colors = Colors(colors.into());
    c.file.clear();
    c.source.clear();
    c.max_points = None;
    c.filters.clear();
    store(s, c, a.name, base, id)
}
