use super::*;
use crate::geom::prelude::*;
use solidcore_record::Plane;
use solidcraft_doc::{PlaneRef, apply_vector};

// Jacobi rotations diagonalize the symmetric 3x3 area covariance.
// Columns of the accumulated rotation matrix are the principal directions.
pub(in crate::cmd) fn principal_axes(mut a: [[f64; 3]; 3]) -> ([f64; 3], [[f64; 3]; 3]) {
    let mut axes = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
    for _ in 0..32 {
        let (p, q) = [(0, 1), (0, 2), (1, 2)].into_iter().max_by(|&(i, j), &(k, l)| a[i][j].abs().total_cmp(&a[k][l].abs())).unwrap_or((0, 1));
        if a[p][q].abs() <= f64::EPSILON * (a[0][0] + a[1][1] + a[2][2]).abs() {
            break;
        }
        let (sin, cos) = (0.5 * (2. * a[p][q]).atan2(a[q][q] - a[p][p])).sin_cos();
        let (pp, qq, pq) = (a[p][p], a[q][q], a[p][q]);
        for k in 0..3 {
            if k != p && k != q {
                let (kp, kq) = (a[k][p], a[k][q]);
                a[k][p] = cos * kp - sin * kq;
                a[p][k] = a[k][p];
                a[k][q] = sin * kp + cos * kq;
                a[q][k] = a[k][q];
            }
            let (u, v) = (axes[k][p], axes[k][q]);
            axes[k][p] = cos * u - sin * v;
            axes[k][q] = sin * u + cos * v;
        }
        a[p][p] = cos * cos * pp - 2. * sin * cos * pq + sin * sin * qq;
        a[q][q] = sin * sin * pp + 2. * sin * cos * pq + cos * cos * qq;
        a[p][q] = 0.;
        a[q][p] = 0.;
    }
    let mut order = [0, 1, 2];
    order.sort_by(|&i, &j| a[i][i].total_cmp(&a[j][j]));
    (order.map(|i| a[i][i]), order.map(|i| [axes[0][i], axes[1][i], axes[2][i]]))
}
#[derive(serde::Serialize, JsonSchema)]
pub(super) struct PlaneResult {
    #[serde(flatten)]
    feature: crate::schema::features::FeatureResult,
    region_triangles: usize,
    rms_mm: f64,
    max_mm: f64,
    fit_report: String,
}

pub(super) fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let id = "mesh.fit_region_plane";
    let args: region::Region = parse(p, id)?;
    let src = source(s, id, args.body.as_deref())?;
    let region=region::members(&args,&src,id,20.)?;
    let point = |i: u32| {
        let n = i as usize * 3;
        let a = &src.mesh.positions;
        Vec3::new(a[n], a[n + 1], a[n + 2])
    };
    let face = |i: usize| {
        let k = i * 3;
        let t = &src.mesh.triangles;
        [point(t[k]), point(t[k + 1]), point(t[k + 2])]
    };
    let [u, v, w] = face(*region.first().ok_or_else(||bad(id,"empty region"))? as usize);
    let seed_normal = (v - u).cross(w - u);
    let mut area = 0.;
    let mut centroid = Vec3::ZERO;
    for &i in &region {
        let [a, b, c] = face(i as usize);
        let weight = (b - a).cross(c - a).len();
        centroid = centroid + ((a - u) + (b - u) + (c - u)) * (weight / 3.);
        area += weight;
    }
    if !area.is_finite() || area <= 0. {
        return Err(bad(id, "region has no finite area"));
    }
    centroid = u + centroid * (1. / area);
    let mut covariance = [[0.; 3]; 3];
    for &i in &region {
        let [a, b, c] = face(i as usize);
        let weight = (b - a).cross(c - a).len() / (12. * area);
        let offsets = [a - centroid, b - centroid, c - centroid];
        // Exact second moments of uniform area on a triangle.
        for v in [offsets[0], offsets[1], offsets[2], offsets[0] + offsets[1] + offsets[2]] {
            let coords = [v.x, v.y, v.z];
            for j in 0..3 {
                for k in 0..3 {
                    covariance[j][k] += weight * coords[j] * coords[k];
                }
            }
        }
    }
    let [x, y, z] = principal_axes(covariance).1[0];
    let mut normal = Vec3::new(x, y, z);
    if normal.dot(seed_normal) < 0. {
        normal = -normal;
    }
    let mut maximum = 0.0_f64;
    let mut variance = 0.;
    for &i in &region {
        let [a, b, c] = face(i as usize);
        let [u, v, w] = [a, b, c].map(|p| (p - centroid).dot(normal));
        maximum = maximum.max(u.abs()).max(v.abs()).max(w.abs());
        variance += (b - a).cross(c - a).len() / area * (u * u + v * v + w * w + (u + v + w).powi(2)) / 12.;
    }
    let rms = variance.sqrt();
    let plane =
        Plane::from_normal(apply_point(&src.frame, centroid), apply_vector(&src.frame, normal)).ok_or_else(|| bad(id, "invalid fitted plane"))?;
    let owner = json!([src.feature,src.index]);
    let mut result = super::super::features::add_feature(
        s,
        &json!({"name":"Fitted scan plane"}),
        FeatureKind::ConstructionPlane { plane: PlaneRef::Custom { plane } },
    )?;
    let feature = result["feature"].as_u64().ok_or_else(||bad(id,"fitted feature id missing"))?;
    s.doc_mut().metadata.0.insert(alignment::plane::owner_key(feature),owner);
    result["region_triangles"] = json!(region.len());
    result["rms_mm"] = json!(rms);
    result["max_mm"] = json!(maximum);
    result["fit_report"] =
        json!(format!("Fitted a construction plane to {} triangles. RMS deviation {rms:.4} mm; maximum {maximum:.4} mm.", region.len()));
    Ok(result)
}
