use super::*;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct Region {
    pub(super) body: Option<String>,
    /// Source triangle index, as an alternative to a viewport pick.
    triangle: Option<usize>,
    /// Zero-based region from Detect Regions, using the same angle.
    auto_region: Option<u32>,
    /// A world point on a scan triangle.
    point: Option<[f64; 3]>,
    /// Maximum angle to the starting facet's normal in degrees (0–180, default 20).
    pub(super) angle: Option<f64>,
}

#[derive(serde::Serialize, JsonSchema)]
pub(super) struct RegionResult {
    body: String,
    region_triangles: usize,
    triangles: Vec<u32>,
}

pub(super) fn seed(args: &Region, source: &Source<'_>, id: &str) -> Result<usize> {
    match (args.triangle, args.point) {
        (Some(index), None) => Ok(index),
        (None, Some([x, y, z])) if x.is_finite() && y.is_finite() && z.is_finite() => {
            let inv = mat_inverse(&source.frame).ok_or_else(|| bad(id, "mesh placement is singular"))?;
            facets::choose_triangle(source.mesh, apply_point(&inv, Vec3::new(x, y, z))).map_err(|e| bad(id, e))
        }
        _ => Err(bad(id, "give one source triangle index or a finite world point")),
    }
}

pub(super) fn members(args: &Region, src: &Source<'_>, id: &str, default_angle: f64) -> Result<Vec<u32>> {
    if let Some(label) = args.auto_region {
        if args.point.is_some() || args.triangle.is_some() {
            return Err(bad(id, "choose either a detected region or a facet"));
        }
        let (labels, groups) = src.mesh.segment_scan(args.angle.unwrap_or(25.)).map_err(|e| bad(id, e.to_string()))?;
        if label as usize >= groups.len() {
            return Err(bad(id, "unknown detected region"));
        }
        return Ok(labels.iter().enumerate().filter_map(|(i, &v)| (v == label).then_some(i as u32)).collect());
    }
    src.mesh.scan_region(seed(args, src, id)?, args.angle.unwrap_or(default_angle)).map_err(|e| bad(id, e.to_string()))
}
impl Session {
    /// Region preview for the desktop viewport. Keep the exact count but bound overlay geometry
    /// and allocations independently of scan size.
    pub fn preview_mesh_region(&self, p: &Value) -> Result<(usize, Vec<[Vec3; 3]>)> {
        let id = "mesh.select_region";
        let args: Region = parse(p, id)?;
        let src = source(self, id, args.body.as_deref())?;
        let triangles = members(&args, &src, id, 20.)?;
        let count = triangles.len();
        let sample = count.min(4096);
        let positions = src.mesh.positions.as_chunks::<3>().0;
        let faces = src.mesh.triangles.as_chunks::<3>().0;
        let overlay = (0..sample)
            .map(|i| {
                let index = i * (count - 1) / sample.saturating_sub(1).max(1);
                faces[triangles[index] as usize].map(|vertex| {
                    let [x, y, z] = positions[vertex as usize];
                    apply_point(&src.frame, Vec3::new(x, y, z))
                })
            })
            .collect();
        Ok((count, overlay))
    }
}

pub(super) fn select_region(s: &mut Session, p: &Value) -> Result<Value> {
    let id = "mesh.select_region";
    let args: Region = parse(p, id)?;
    let src = source(s, id, args.body.as_deref())?;
    let triangles = members(&args, &src, id, 20.)?;
    Ok(json!(RegionResult { body: src.name, region_triangles: triangles.len(), triangles }))
}

fn edit_region(s: &mut Session, p: &Value, id: &str, isolate: bool) -> Result<Value> {
    let args: Region = parse(p, id)?;
    if args.auto_region.is_some() {
        let indices = members(&args, &source(s, id, args.body.as_deref())?, id, 25.)?;
        let mut ranges: Vec<[u32; 2]> = Vec::new();
        for i in indices {
            if let Some(last) = ranges.last_mut()
                && last[1].checked_add(1) == Some(i)
            {
                last[1] = i;
            } else {
                ranges.push([i, i]);
            }
        }
        return edit(s, id, args.body.as_deref(), |m, _| m.retain_scan_facets(&ranges, isolate).map_err(|e| bad(id, e.to_string())));
    }
    let index = seed(&args, &source(s, id, args.body.as_deref())?, id)?;
    edit(s, id, args.body.as_deref(), |m, _| m.edit_scan_region(index, args.angle.unwrap_or(20.0), isolate).map_err(|e| bad(id, e.to_string())))
}

pub(super) fn delete_region(s: &mut Session, p: &Value) -> Result<Value> {
    edit_region(s, p, "mesh.delete_region", false)
}

pub(super) fn isolate_region(s: &mut Session, p: &Value) -> Result<Value> {
    edit_region(s, p, "mesh.isolate_region", true)
}
