use super::{MeshData, Result, invalid, picked_island::connected};
use solidcore_record::Vec3;
use solidcraft_geom::Vec3Geom;

pub(super) fn face_normal(points: &[[f64; 3]], face: [u32; 3]) -> Option<Vec3> {
    let [a, b, c] = face.map(|id| {
        let [x, y, z] = points[id as usize];
        Vec3::new(x, y, z)
    });
    let n = (b - a).cross(c - a);
    let len = n.len();
    (len > 0.0 && len.is_finite()).then(|| n / len)
}

impl MeshData {
    /// Grow from one facet across entire shared edges. Compare geometric, oriented normals to
    /// the seed throughout the flood, so gradual curvature cannot leak past the user's angle.
    pub fn scan_region(&self, picked: usize, degrees: f64) -> Result<Vec<u32>> {
        if !(0.0..=180.0).contains(&degrees) {
            return Err(invalid("normal angle must be 0 to 180 degrees"));
        }
        let (points, faces) = self.scan_arrays()?;
        let face = *faces.get(picked).ok_or_else(|| invalid("source triangle index out of range"))?;
        let seed = face_normal(points, face).ok_or_else(|| invalid("the seed triangle is degenerate"))?;
        let threshold = degrees.to_radians().cos() - 8.0 * f64::EPSILON;
        connected(points, faces, picked, |i| face_normal(points, faces[i]).is_some_and(|n| n.dot(seed) >= threshold))
    }

    /// Apply the same geometric region as the read-only selection, keeping per-corner normals.
    pub fn edit_scan_region(&mut self, picked: usize, degrees: f64, isolate: bool) -> Result<()> {
        let chosen = self.scan_region(picked, degrees)?;
        self.retain_scan_region(&chosen, isolate)
    }
}
