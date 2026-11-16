use super::*;
const ID: &str = "mesh.auto_regions";
const MAX_ROWS: usize = 4096;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct Args {
    body: Option<String>,
    /// Maximum local dihedral angle across a manifold edge (0–180 degrees, default 25).
    angle: Option<f64>,
    /// Include dense 8-bit palette indices for the full-resolution displayed mesh.
    palette: Option<bool>,
    /// Return all source triangle indices belonging to this detected region.
    region: Option<u32>,
}
#[derive(serde::Serialize, JsonSchema)]
struct Patch {
    region: u32,
    seed: u32,
    seed_point: [f64; 3],
    triangles: u32,
}
#[derive(serde::Serialize, JsonSchema)]
struct Sample {
    region: u32,
    triangle: u32,
    points: [[f64; 3]; 3],
}
#[derive(serde::Serialize, JsonSchema)]
pub(super) struct Output {
    angle: f64,
    body: String,
    triangles: usize,
    region_count: usize,
    degenerate_triangles: usize,
    regions: Vec<Patch>,
    regions_truncated: bool,
    overlay: Vec<Sample>,
    regions_report: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    palette_indices_b64: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    members: Option<Vec<u32>>,
}

impl Session {
    /// Native ids index the displayed mesh: its importer removes repeated-position corners
    /// but preserves the order of surviving triangles, including collinear triangles.
    pub fn auto_mesh_regions(&self, p: &Value) -> Result<(Value, Vec<u32>)> {
        let args: Args = parse(p, ID)?;
        let src = source(self, ID, args.body.as_deref())?;
        let (ids, groups) = src.mesh.segment_scan(args.angle.unwrap_or(25.)).map_err(|e| bad(ID, e.to_string()))?;
        if args.region.is_some_and(|i| i as usize >= groups.len()) {
            return Err(bad(ID, "detected region number is out of range"));
        }
        let pts = src.mesh.positions.as_chunks::<3>().0;
        let faces = src.mesh.triangles.as_chunks::<3>().0;
        let world = |i: u32| {
            faces[i as usize].map(|v| {
                let [x, y, z] = pts[v as usize];
                let p = apply_point(&src.frame, Vec3::new(x, y, z));
                [p.x, p.y, p.z]
            })
        };
        let mut rows = Vec::with_capacity(groups.len().min(MAX_ROWS));
        for (id, &(seed, triangles)) in groups.iter().take(MAX_ROWS).enumerate() {
            let [a, b, c] = world(seed);
            rows.push(Patch { region: id as u32, seed, triangles, seed_point: std::array::from_fn(|i| (a[i] + b[i] + c[i]) / 3.) });
        }
        let mut overlay = Vec::new();
        let samples = faces.len().min(MAX_ROWS);
        for i in 0..samples {
            let j = i * (faces.len() - 1) / samples.saturating_sub(1).max(1);
            if ids[j] != u32::MAX {
                overlay.push(Sample { region: ids[j], triangle: j as u32, points: world(j as u32) });
            }
        }
        let members = args.region.map(|region| ids.iter().enumerate().filter_map(|(i, &id)| (id == region).then_some(i as u32)).collect());
        let degenerate_triangles = ids.iter().filter(|&&id| id == u32::MAX).count();
        let display = if degenerate_triangles == 0 {
            ids
        } else {
            ids.into_iter()
                .enumerate()
                .filter_map(|(i, id)| {
                    let [a, b, c] = faces[i].map(|v| pts[v as usize]);
                    (a != b && b != c && c != a).then_some(id)
                })
                .collect()
        };
        let palette_indices_b64 = args.palette.unwrap_or(false).then(|| {
            let palette = display.iter().map(|&v| if v == u32::MAX { 0 } else { (1 + v % 255) as u8 }).collect::<Vec<_>>();
            solidcraft_doc::canvas::base64_encode(&palette)
        });
        let regions_report = format!(
            "Detected {} regions across {} facets; {} degenerate facets excluded.",
            groups.len(),
            faces.len() - degenerate_triangles,
            degenerate_triangles
        );
        Ok((
            json!(Output {
                body: src.name,
                angle: args.angle.unwrap_or(25.),
                triangles: faces.len(),
                region_count: groups.len(),
                degenerate_triangles,
                regions: rows,
                regions_truncated: groups.len() > MAX_ROWS,
                overlay,
                regions_report,
                palette_indices_b64,
                members
            }),
            display,
        ))
    }
}

pub(super) fn run(s: &mut Session, p: &Value) -> Result<Value> {
    s.auto_mesh_regions(p).map(|(report, _)| report)
}
