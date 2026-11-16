use super::{fit_plane::principal_axes, region, *};
use crate::geom::prelude::*;
use solidcore_record::Plane;
use solidcraft_doc::{Dir, Operation, apply_vector, point_expr};

#[derive(serde::Serialize, JsonSchema)]
pub(super) struct CylinderResult {
    #[serde(flatten)]
    feature: crate::schema::features::FeatureResult,
    region_triangles: usize,
    radius_mm: f64,
    height_mm: f64,
    rms_mm: f64,
    max_mm: f64,
    fit_report: String,
}

pub(super) fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let id = "mesh.fit_region_cylinder";
    let args: region::Region = parse(p, id)?;
    let src = source(s, id, args.body.as_deref())?;
    let mesh = src.mesh;
    let patch=region::members(&args,&src,id,65.)?;
    let vertex = |v: u32| {
        let k = v as usize * 3;
        Vec3::new(mesh.positions[k], mesh.positions[k + 1], mesh.positions[k + 2])
    };
    let face = |i: u32| {
        let k = i as usize * 3;
        [mesh.triangles[k], mesh.triangles[k + 1], mesh.triangles[k + 2]].map(vertex)
    };
    // Facet normals on a cylinder lie in a plane perpendicular to its axis.
    let mut moments = [[0.; 3]; 3];
    let mut weight = 0.;
    let reference = face(patch[0])[0];
    let mut center = Vec3::ZERO;
    for &i in &patch {
        let [a, b, c] = face(i);
        let cross = (b - a).cross(c - a);
        let area = cross.len();
        if area <= 0. {
            continue;
        }
        center = center + ((a - reference) + (b - reference) + (c - reference)) * (area / 3.);
        weight += area;
        let n = [cross.x, cross.y, cross.z];
        for j in 0..3 {
            for k in 0..3 {
                moments[j][k] += n[j] * n[k] / area;
            }
        }
    }
    let (values, axes) = principal_axes(moments);
    if weight <= 0. || values[1] < weight * 0.002 || values[0] > values[1] * 0.12 {
        return Err(bad(
            id,
            "Pick a curved cylindrical side patch with a wider spread of normals. Adjust the region angle to exclude caps or adjacent faces.",
        ));
    }
    center = reference + center * (1. / weight);
    let [x, y, z] = axes[0];
    let axis = Vec3::new(x, y, z);
    let plane = Plane::from_normal(center, axis).ok_or_else(|| bad(id, "invalid cylinder axis"))?;
    let mut xx = 0.;
    let mut yy = 0.;
    let mut xy = 0.;
    let mut xr = 0.;
    let mut yr = 0.;
    let mut r2 = 0.;
    // The area-weighted vertex centroid above centers the 2D circle normal equations.
    for &i in &patch {
        let f = face(i);
        let w = (f[1] - f[0]).cross(f[2] - f[0]).len() / (3. * weight);
        for point in f {
            let p = point - center;
            let u = p.dot(plane.x);
            let v = p.dot(plane.y);
            let r = u * u + v * v;
            xx += w * u * u;
            yy += w * v * v;
            xy += w * u * v;
            xr += w * u * r;
            yr += w * v * r;
            r2 += w * r;
        }
    }
    let det = xx * yy - xy * xy;
    if !(det > f64::EPSILON * (xx + yy).powi(2)) {
        return Err(bad(id, "circular cross-section could not be resolved"));
    }
    let cx = (yy * xr - xy * yr) / (2. * det);
    let cy = (xx * yr - xy * xr) / (2. * det);
    let radius = (r2 + cx * cx + cy * cy).sqrt();
    let origin = center + plane.x * cx + plane.y * cy;
    let mut low = f64::INFINITY;
    let mut high = f64::NEG_INFINITY;
    let mut variance = 0.;
    let mut maximum = 0.0_f64;
    for &i in &patch {
        let f = face(i);
        let w = (f[1] - f[0]).cross(f[2] - f[0]).len() / (3. * weight);
        for point in f {
            let p = point - origin;
            let h = p.dot(axis);
            let deviation = (p - axis * h).len() - radius;
            low = low.min(h);
            high = high.max(h);
            maximum = maximum.max(deviation.abs());
            variance += w * deviation * deviation;
        }
    }
    let rms = variance.sqrt();
    let height = high - low;
    if !(radius.is_finite() && height > 1e-9 && radius > 1e-9 && rms <= radius * 0.1) {
        return Err(bad(id, "selected patch does not define a stable cylinder"));
    }
    let base = apply_point(&src.frame, origin + axis * low);
    let world_axis = apply_vector(&src.frame, axis);
    let kind = FeatureKind::Cylinder {
        base: point_expr(base),
        axis: Dir(world_axis),
        radius: format!("{radius} mm").into(),
        height: format!("{height} mm").into(),
        operation: Operation::NewBody,
        references: Vec::new(),
    };
    let mut result = super::super::features::add_feature(s, &json!({"name":"Fitted scan cylinder"}), kind)?;
    result["region_triangles"] = json!(patch.len());
    result["radius_mm"] = json!(radius);
    result["height_mm"] = json!(height);
    result["rms_mm"] = json!(rms);
    result["max_mm"] = json!(maximum);
    result["fit_report"] = json!(format!(
        "Fitted a cylinder to {} triangles: radius {radius:.4} mm, height {height:.4} mm. Radial error at vertices: RMS {rms:.4} mm, maximum {maximum:.4} mm.",
        patch.len()
    ));
    Ok(result)
}
