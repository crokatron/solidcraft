//! Exact geometric samples and rigid handle-frame placement. No display meshes.
use super::api_queries as q;
use crate::geom::object::Shape;
use crate::nurbs::b_spline_surface_n_lib::p2::OrientType;
use crate::query::brep::{BrepQuery, LiveBrepObjects};
use crate::topology::{Brep, BrepId, EdgeId, FaceId, FaceuseId};
use crate::{Kernel, Result, SolidError, Vec3};
const OP: &str = "geometry_snap";
fn bad(reason: &'static str) -> SolidError {
    SolidError::invalid_input(OP, reason)
}
fn v(p: [f64; 3]) -> Result<Vec3> {
    if p.iter().all(|x| x.is_finite() && x.abs() <= 1e9) {
        Ok(Vec3::new(p[0], p[1], p[2]))
    } else {
        Err(bad("non-finite or out-of-range coordinate"))
    }
}
fn a(p: Vec3) -> [f64; 3] {
    [p.x, p.y, p.z]
}
/// A point on exact geometry and its unit direction (normal or tangent).
#[derive(Clone, Copy, Debug)]
pub struct Sample {
    /// Point on the exact curve or trimmed face.
    pub point: [f64; 3],
    /// Unit tangent for an edge, outward normal for a face.
    pub direction: [f64; 3],
}
/// Sample an edge at the nearest point or half its arc length, using the exact curve.
///
/// # Errors
/// Invalid input, degenerate tangent, failed projection or an arc-length solve that does not converge.
pub fn edge_sample(k: &Kernel, b: &Brep, e: EdgeId, p: [f64; 3], midpoint: bool) -> Result<Sample> {
    let hint = v(p)?;
    let (point, t) = if midpoint {
        let interval = q::edge_parameter_range(b, e)?;
        let curve = crate::topology::edge::evaluation_curve(b, e)?
            .ok_or_else(|| bad("edge has no curve"))?;
        let length = curve.length(k, &interval, 1e-9)?;
        if !length.is_finite() || length <= 0.0 {
            return Err(bad("edge has no finite positive length"));
        }
        let middle = curve.find_parameter_at_arc_length(k, interval.min(), length * 0.5)?;
        if !middle.converged || middle.past_end || !interval.contains_value(middle.end_param, 1e-9)
        {
            return Err(bad("edge arc-length midpoint did not converge"));
        }
        let t = middle.end_param;
        (curve.evaluate_point(k, t)?, t)
    } else {
        let c = q::edge_closest_point(k, b, e, hint)?;
        (c.closest.point, c.parameter)
    };
    Ok(Sample {
        point: a(point),
        direction: a(q::edge_tangent(k, b, e, t)?),
    })
}
/// Project onto a trimmed face and return its local outward unit normal.
///
/// # Errors
/// Invalid input, failed projection, singular normal or an unknown face-use orientation.
pub fn face_sample(
    k: &mut Kernel,
    b: &mut Brep,
    f: FaceId,
    fu: FaceuseId,
    p: [f64; 3],
) -> Result<Sample> {
    let hint = v(p)?;
    let id = BrepId::from_index(0);
    let shape = Shape::new(k, &(id, &*b), &[], &[(id, f)], &[], &[])?;
    let query = BrepQuery::of_brep(b);
    let identity = query.effective_identity();
    let objects = LiveBrepObjects::new(b, &identity, None)?;
    let point = q::face_closest_point_in_shape(k, &shape, &objects, (id, f), hint)?.point;
    drop(objects);
    let surface = b
        .face(f)?
        .surface
        .as_ref()
        .ok_or_else(|| bad("face has no surface"))?;
    let projected = q::surface_closest_point(k, surface, point)?;
    let uv = *projected
        .uvs
        .first()
        .ok_or_else(|| bad("surface has no parameter at point"))?;
    let normal = crate::query::brep::evaluate_surface_normal(k, Some(surface), uv)?.unitize()?;
    let normal = match b.faceuse(fu)?.orientation {
        OrientType::Same => normal,
        OrientType::Opposite => -normal,
        _ => return Err(bad("face has unknown orientation")),
    };
    Ok(Sample {
        point: a(point),
        direction: a(normal),
    })
}
/// Midpoint of a cylindrical or conical face's bounded axial extent.
/// The caller supplies the analytic carrier frame. Height has no interior extrema
/// on these carriers, so exact trimmed boundary curves determine both ends.
/// No display mesh or unbounded carrier origin is used as the centre.
///
/// # Errors
/// Invalid frame, missing face boundaries or unsupported exact curve bounds.
pub fn face_axis_center(
    k: &Kernel,
    b: &Brep,
    f: FaceId,
    origin: [f64; 3],
    x: [f64; 3],
    y: [f64; 3],
) -> Result<Sample> {
    use crate::geom::axis2_placement::Axis2Placement;
    let mut frame = Axis2Placement::IDENTITY;
    frame.set_canonical(v(origin)?, v(x)?, v(y)?)?;
    let inverse = frame.invert();
    let mut low = f64::INFINITY;
    let mut high = f64::NEG_INFINITY;
    for edge in crate::topology::topology_traverser::face_edges(b, f)? {
        let mut curve = crate::topology::edge::evaluation_curve(b, edge)?
            .ok_or_else(|| bad("face boundary has no exact curve"))?;
        curve.transform(k, &inverse, None)?;
        let bounds = curve.calculate_tight_bounding_box(k, &q::edge_parameter_range(b, edge)?)?;
        low = low.min(bounds.min().z);
        high = high.max(bounds.max().z);
    }
    if !low.is_finite() || !high.is_finite() || low > high {
        return Err(bad("face has no finite axial bounds"));
    }
    let point = frame.transform_point(Vec3::new(0., 0., low * 0.5 + high * 0.5));
    Ok(Sample {
        point: a(point),
        direction: a(frame.z_axis().unitize()?),
    })
}
/// Intersection of extended straight lines. Parallel and skew lines are refused.
pub fn line_intersection(
    first: [[f64; 3]; 2],
    second: [[f64; 3]; 2],
    tolerance: f64,
) -> Result<Sample> {
    if !tolerance.is_finite() || tolerance <= 0. || tolerance > 1. {
        return Err(bad("invalid intersection tolerance"));
    }
    let (p, q) = (v(first[0])?, v(second[0])?);
    let (d, e) = (
        (v(first[1])? - p).unitize()?,
        (v(second[1])? - q).unitize()?,
    );
    if d.cross(e).length() <= 1e-10 {
        return Err(bad("parallel lines have no unique intersection"));
    }
    let (t, u) = crate::geom::geom_utility::p1::line_line_closest_point(p, d, q, e)?;
    let (x, y) = (p + d * t, q + e * u);
    if x.distance(y) > tolerance {
        return Err(bad("skew lines do not intersect in 3D"));
    }
    Ok(Sample {
        point: a((x + y) * 0.5),
        direction: a(d),
    })
}
/// Normal of a circular frame, calculated from its exact defining axes.
pub fn frame_normal(x: [f64; 3], y: [f64; 3]) -> Result<[f64; 3]> {
    Ok(a(v(x)?.cross(v(y)?).unitize()?))
}
/// Rotate a right-handed frame by the shortest rotation taking one axis onto direction.
/// The opposite-axis case rotates around the next frame axis, retaining a deterministic roll.
pub fn align_axis(
    frame: &[[f64; 4]; 4],
    axis: usize,
    direction: [f64; 3],
    flip: bool,
) -> Result<[[f64; 4]; 4]> {
    if axis > 2 || !frame.iter().flatten().all(|x| x.is_finite()) {
        return Err(bad("invalid frame or axis"));
    }
    let axes = [
        v([frame[0][0], frame[0][1], frame[0][2]])?,
        v([frame[1][0], frame[1][1], frame[1][2]])?,
        v([frame[2][0], frame[2][1], frame[2][2]])?,
    ];
    if axes.iter().any(|x| (x.length() - 1.).abs() > 1e-6)
        || axes[0].dot(axes[1]).abs() > 1e-6
        || axes[0].dot(axes[2]).abs() > 1e-6
        || axes[1].dot(axes[2]).abs() > 1e-6
        || axes[0].cross(axes[1]).dot(axes[2]) < 0.999999
    {
        return Err(bad("frame must be orthonormal and right handed"));
    }
    let target = v(direction)?.unitize()? * if flip { -1. } else { 1. };
    let from = axes[axis];
    let c = from.dot(target).clamp(-1., 1.);
    let cross = from.cross(target);
    let s = cross.length();
    let rotate = |x: Vec3| -> Vec3 {
        if s > 1e-12 {
            let k = cross / s;
            x * c + k.cross(x) * s + k * (k.dot(x) * (1. - c))
        } else if c < 0. {
            let k = axes[(axis + 1) % 3];
            k * (2. * k.dot(x)) - x
        } else {
            x
        }
    };
    let mut out = *frame;
    for i in 0..3 {
        let r = a(rotate(axes[i]));
        out[i] = [r[0], r[1], r[2], 0.];
    }
    Ok(out)
}
