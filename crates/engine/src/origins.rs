//! The reference geometry of an exact assembly placement, without timeline features.

use solidcore_record::{Plane, Vec3};
use solidcraft_doc::construct::{ConstructGeom, DatumItem, DatumRef};

use crate::{Result, Session};
use solidcraft_doc::{IDENTITY, Mat, mat_inverse, mat_mul};
use solidcraft_geom::prelude::*;

pub const ITEMS: [&str; 7] = ["O", "X", "Y", "Z", "XY", "XZ", "YZ"];
pub const MAX_SHOWN: usize = 10_000;

/// A component Origin folder uses the same scope as its Bodies and Sketches folders.
pub fn folder(path: &[u64]) -> String {
    format!("{}/origin", Session::folder_scope(path))
}

/// Parse a bounded, canonical placement folder key. The design's origin has its own controls.
pub fn folder_path(key: &str) -> Option<Vec<u64>> {
    if key.len() > 2048 {
        return None;
    }
    let ids = key.strip_prefix("o:")?.strip_suffix("/origin")?;
    if ids.is_empty() || ids.split('/').count() > 64 {
        return None;
    }
    ids.split('/').map(|id| id.parse::<u64>().ok().filter(|n| *n != 0 && n.to_string() == id)).collect()
}

/// An item key carries the complete path: a shared child in two parents has two origins. The
/// item reads the way the document's datum grammar parses it — case-insensitively, answered in
/// the canonical spelling (`yz` answers `YZ`) — so a resolver that takes a key in any spelling
/// and one that matches the stored spelling agree about what a key names, and a key no datum
/// grammar accepts is no datum anywhere (#7313).
pub fn item(name: &str) -> Option<(Vec<u64>, &'static str)> {
    let (key, item) = name.rsplit_once('/')?;
    let item = ITEMS.iter().find(|known| known.eq_ignore_ascii_case(item))?;
    Some((folder_path(key)?, *item))
}

/// The Browser key of a placement's origin item: the folder path, then the item
/// (`o:2/origin/XY`). Resolvers and the saved visibility speak these keys.
pub fn key(path: &[u64], item: &str) -> String {
    format!("{}/{item}", folder(path))
}

/// The datum an item key names, by the document's shared datum grammar: `o:<ids>/origin/<item>`
/// with its strict bounds. Resolvers save it, so the reference follows the placement.
pub fn datum(name: &str) -> Option<DatumRef> {
    DatumRef::parse_key(name)
}

impl Session {
    /// Resolve a Browser pick even when its origin is hidden. Includes pending placement moves.
    pub fn origin_geometry(&self, name: &str) -> Option<ConstructGeom> {
        let d = datum(name)?;
        let placement = self.occurrence_placement(&d.path).ok()?;
        Some(d.item.geom().placed(&placement.world))
    }

    /// The plane a component origin key names, placed where its component now stands:
    /// `None` for the axes and the point, and for a key whose placement is gone.
    pub fn origin_plane(&self, name: &str) -> Option<Plane> {
        match self.origin_geometry(name)? {
            ConstructGeom::Plane(p) => Some(p),
            _ => None,
        }
    }

    /// The axis a component origin key names, placed where its component now stands:
    /// `None` for the planes and the point, and for a key whose placement is gone.
    pub fn origin_axis(&self, name: &str) -> Option<(Vec3, Vec3)> {
        match self.origin_geometry(name)? {
            ConstructGeom::Axis { origin, dir } => Some((origin, dir)),
            _ => None,
        }
    }

    /// Reference geometry explicitly shown by the user. No assembly expansion on every frame.
    pub fn shown_component_origins(&self) -> Vec<(String, ConstructGeom)> {
        let mut out = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for path in self.visibility.shown_component_origins.iter().take(MAX_SHOWN) {
            if path.is_empty() || path.len() > 64 || !seen.insert(path) || !self.occurrence_visible(path) {
                continue;
            }
            let Ok(placement) = self.occurrence_placement(path) else { continue };
            for item in ITEMS {
                let name = key(path, item);
                if !self.visibility.hidden_origin.contains(&name)
                    && let Some(geom) = DatumItem::parse(item).map(|d| d.geom())
                {
                    out.push((name, geom.placed(&placement.world)));
                }
            }
        }
        out
    }
}

/// A complete coordinate frame, resolved in world space at the current placement.
/// Keys identify references; matrices are snapshots, not constraints or saved datums.
#[derive(Clone, Debug, serde::Serialize, schemars::JsonSchema)]
pub struct CoordinateFrame {
    pub key: String,
    pub label: String,
    /// Column-major affine matrix: X, Y, Z axes and origin.
    pub matrix: Mat,
}

/// A rigid delta that carries every axis and the origin of `source` onto `target`.
/// Unlike normal-only alignment this preserves the specified roll about Z.
pub fn frame_delta(source: &Mat, target: &Mat) -> Result<Mat> {
    let fail = || crate::EngineError::Other("Coordinate systems must have finite origins and orthonormal right-handed axes.".into());
    if !solidcraft_doc::is_placement(source) || !solidcraft_doc::is_placement(target) {
        return Err(fail());
    }
    let inverse = mat_inverse(source).ok_or_else(fail)?;
    let delta = mat_mul(target, &inverse);
    if !solidcraft_doc::is_placement(&delta) {
        return Err(fail());
    }
    Ok(delta)
}

fn plane_frame(plane: Plane) -> Mat {
    let z = plane.normal();
    [
        [plane.x.x, plane.x.y, plane.x.z, 0.],
        [plane.y.x, plane.y.y, plane.y.z, 0.],
        [z.x, z.y, z.z, 0.],
        [plane.origin.x, plane.origin.y, plane.origin.z, 1.],
    ]
}

impl Session {
    /// Full world, component-origin, named-origin and construction-plane coordinate systems.
    /// Includes pending occurrence placements, without changing the document or the undo stack.
    pub fn coordinate_frames(&self) -> Vec<CoordinateFrame> {
        let mut out = vec![CoordinateFrame { key: "world".into(), label: "World CSYS".into(), matrix: IDENTITY }];
        let doc = self.placed_document();
        let state = self.model.state();
        let mut planes = std::collections::BTreeMap::<u64, Vec<_>>::new();
        for construct in &state.construct {
            let ConstructGeom::Plane(plane) = construct.geom else { continue };
            let Some(feature) = doc.feature(construct.feature) else { continue };
            if feature.suppressed {
                continue;
            }
            planes.entry(feature.component).or_default().push((construct, plane));
        }
        let mut named = std::collections::BTreeMap::<u64, Vec<_>>::new();
        for entry in &doc.assembly.origins {
            if let Ok(local) = solidcraft_doc::joints::resolve_origin(&doc, &state, &entry.origin) {
                named.entry(entry.origin.occurrence).or_default().push((entry, local));
            }
        }
        let occurrences: std::collections::BTreeMap<_, _> = doc.occurrences.iter().map(|o| (o.id, o)).collect();
        for (component, path, world) in doc.placements() {
            if out.len() >= MAX_SHOWN {
                break;
            }
            let suffix = path.iter().filter_map(|id| occurrences.get(id).map(|o| o.name.as_str())).collect::<Vec<_>>().join(" / ");
            if component != 0 {
                out.push(CoordinateFrame { key: folder(&path), label: format!("{suffix} - Origin CSYS"), matrix: world });
            }
            let path_key = if path.is_empty() { "root".into() } else { path.iter().map(u64::to_string).collect::<Vec<_>>().join("/") };
            for (entry, local) in named.get(&path.last().copied().unwrap_or(0)).into_iter().flatten() {
                if out.len() >= MAX_SHOWN {
                    break;
                }
                let matrix = mat_mul(&world, local);
                if !solidcraft_doc::is_placement(&matrix) {
                    continue;
                }
                out.push(CoordinateFrame {
                    key: format!("joint:{}:{path_key}", entry.name),
                    label: if suffix.is_empty() { format!("{} - Named CSYS", entry.name) } else { format!("{suffix} / {} - Named CSYS", entry.name) },
                    matrix,
                });
            }
            for (construct, plane) in planes.get(&component).into_iter().flatten() {
                if out.len() >= MAX_SHOWN {
                    break;
                }
                let matrix = mat_mul(&world, &plane_frame(*plane));
                if !solidcraft_doc::is_placement(&matrix) {
                    continue;
                }
                let path_key = if path.is_empty() { "root".into() } else { path.iter().map(u64::to_string).collect::<Vec<_>>().join("/") };
                out.push(CoordinateFrame {
                    key: format!("plane:{}:{path_key}", construct.feature),
                    label: if suffix.is_empty() {
                        format!("{} - Plane CSYS", construct.name)
                    } else {
                        format!("{suffix} / {} - Plane CSYS", construct.name)
                    },
                    matrix,
                });
            }
        }
        out
    }

    /// Resolve a previously chosen frame again; missing/suppressed/deleted references fail
    /// instead of silently falling back to the world coordinate system.
    pub fn coordinate_frame(&self, key: &str) -> Result<Mat> {
        if key.len() > 2048 {
            return Err(crate::EngineError::Other("Coordinate system reference is too long.".into()));
        }
        self.coordinate_frames()
            .into_iter()
            .find(|frame| frame.key == key)
            .map(|frame| frame.matrix)
            .ok_or_else(|| crate::EngineError::Other("The selected coordinate system is no longer available. Select it again.".into()))
    }
}
