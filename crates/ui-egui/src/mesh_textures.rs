//! Authored mesh appearance prepared once beside the window and cached by immutable inputs.
use crate::gpu::{SCAN_VERTEX_SIZE, ScanImage, ScanMesh, ScanPlacement};
use solidcraft_engine::geom::{Mesh, mesh_appearance::MeshAppearance};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, LazyLock, Mutex};

type Ready = Arc<Vec<Arc<ScanMesh>>>;
type Entry = (Arc<Mesh>, Arc<MeshAppearance>, Option<Ready>);
#[derive(Clone, Default)]
pub(crate) struct MeshTextures(Arc<Mutex<State>>);
#[derive(Default)]
struct State {
    entries: HashMap<(usize, usize), Entry>,
    order: VecDeque<(usize, usize)>,
    making: usize,
    generation: u64,
    error: Option<String>,
}
impl MeshTextures {
    pub fn warm(&self, mesh: &Arc<Mesh>, appearance: &Arc<MeshAppearance>) {
        let _ = self.get(mesh, appearance);
    }
    pub fn generation(&self) -> u64 {
        self.0.lock().map_or(0, |s| s.generation)
    }
    pub fn take_error(&self) -> Option<String> {
        self.0.lock().ok()?.error.take()
    }
    pub fn busy(&self) -> bool {
        self.0.lock().is_ok_and(|s| s.making != 0)
    }
    pub fn append(&self, out: &mut crate::gpu::GpuScene, mesh: &Arc<Mesh>, appearance: &Arc<MeshAppearance>, place: &solidcraft_engine::doc::Mat) {
        let Some(ready) = self.get(mesh, appearance) else { return };
        let rows = [0, 1, 2].map(|r| [place[0][r] as f32, place[1][r] as f32, place[2][r] as f32, place[3][r] as f32]);
        out.textured.extend(ready.iter().map(|mesh| ScanPlacement { mesh: mesh.clone(), rows }));
    }
    fn get(&self, mesh: &Arc<Mesh>, appearance: &Arc<MeshAppearance>) -> Option<Ready> {
        let key = (Arc::as_ptr(mesh) as usize, Arc::as_ptr(appearance) as usize);
        let mut state = self.0.lock().ok()?;
        if let Some(ready) = state.entries.get(&key).map(|entry| entry.2.clone()) {
            state.order.retain(|old| *old != key);
            state.order.push_back(key);
            return ready;
        }
        #[cfg(target_arch = "wasm32")]
        {
            let ready = match make(mesh, appearance) {
                Ok(ready) => Arc::new(ready),
                Err(error) => {
                    state.error = Some(error);
                    Arc::new(Vec::new())
                }
            };
            state.entries.insert(key, (mesh.clone(), appearance.clone(), Some(ready.clone())));
            return Some(ready);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            if state.making >= 2 {
                return None;
            }
            // Evict only the least recently used completed representation. Clearing all
            // entries on the fifth edit discarded the immediate undo states as well.
            if state.entries.len() >= 4 {
                if let Some(old) = state.order.iter().copied().find(|old| state.entries.get(old).is_some_and(|e| e.2.is_some())) {
                    state.entries.remove(&old);
                    state.order.retain(|key| *key != old);
                }
            }
            state.order.push_back(key);
            state.entries.insert(key, (mesh.clone(), appearance.clone(), None));
            state.making += 1;
            let (shared, mesh, appearance) = (self.0.clone(), mesh.clone(), appearance.clone());
            let started = std::thread::Builder::new().name("scan-textures".into()).spawn(move || {
                let prepared = make(&mesh, &appearance);
                if let Ok(mut state) = shared.lock() {
                    let ready = match prepared {
                        Ok(ready) => ready,
                        Err(error) => {
                            state.error = Some(error);
                            Vec::new()
                        }
                    };
                    if let Some(entry) = state.entries.get_mut(&key) {
                        entry.2 = Some(Arc::new(ready));
                    }
                    state.making = state.making.saturating_sub(1);
                    state.generation = state.generation.wrapping_add(1);
                }
            });
            if let Err(error) = started {
                state.error = Some(error.to_string());
                state.making = state.making.saturating_sub(1);
                if let Some(entry) = state.entries.get_mut(&key) {
                    entry.2 = Some(Arc::new(Vec::new()));
                }
            }
            None
        }
    }
}
pub(crate) fn make(mesh: &Mesh, appearance: &MeshAppearance) -> Result<Vec<Arc<ScanMesh>>, String> {
    let count = mesh.triangles.len();
    if (!appearance.corner_uvs.is_empty() && appearance.corner_uvs.len() != count)
        || (!appearance.corner_colors.is_empty() && appearance.corner_colors.len() != count)
        || (!appearance.triangle_materials.is_empty() && appearance.triangle_materials.len() != count)
    {
        return Err("scan appearance does not match its triangles".into());
    }
    let material_count = appearance.materials.len().max(1);
    let mut counts = vec![0usize; material_count];
    for i in 0..count {
        let m = appearance.triangle_materials.get(i).copied().unwrap_or(0) as usize;
        let n = counts.get_mut(m).ok_or("scan material index out of range")?;
        *n += 1;
    }
    let mut groups: Vec<Vec<u8>> = Vec::with_capacity(material_count);
    for n in &counts {
        let mut bytes = Vec::new();
        let len = n.checked_mul(3 * SCAN_VERTEX_SIZE).ok_or("scan vertex allocation overflow")?;
        bytes.try_reserve_exact(len).map_err(|_| "cannot reserve scan texture vertices")?;
        groups.push(bytes);
    }
    for (face, indices) in mesh.triangles.iter().enumerate() {
        let m = appearance.triangle_materials.get(face).copied().unwrap_or(0) as usize;
        let factor = appearance.materials.get(m).map_or([1.0; 4], |m| m.base_color);
        let bytes = groups.get_mut(m).ok_or("scan material index out of range")?;
        for (corner, index) in indices.iter().enumerate() {
            let p = mesh.positions.get(*index as usize).ok_or("scan vertex index out of range")?;
            let uv = appearance.corner_uvs.get(face).map_or([0.0; 2], |u| u[corner]);
            let color = appearance.corner_colors.get(face).map_or([255; 4], |c| c[corner]);
            for v in [p.x as f32, p.y as f32, p.z as f32, uv[0], 1.0 - uv[1]] {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
            bytes.extend((0..4).map(|c| (f32::from(color[c]) * factor[c]).round().clamp(0.0, 255.0) as u8));
        }
    }
    let white = Arc::new(ScanImage { size: [1, 1], rgba: Arc::new(vec![255; 4]) });
    let mut result = Vec::new();
    for (index, vertices) in groups.into_iter().enumerate().filter(|(_, v)| !v.is_empty()) {
        let image = match appearance.materials.get(index).and_then(|m| m.texture.as_ref()) {
            None => white.clone(),
            Some(texture) => decoded_image(&texture.bytes)?,
        };
        result.push(Arc::new(ScanMesh { vertices, image }));
    }
    Ok(result)
}

/// Keep decoded photographs independently of geometry. Every mesh edit preserves its encoded
/// image Arc; decoding the same multi-megapixel JPEG again cannot improve its pixels.
#[derive(Default)]
struct Images {
    entries: VecDeque<(Arc<[u8]>, Arc<ScanImage>)>,
    bytes: usize,
}
static IMAGES: LazyLock<Mutex<Images>> = LazyLock::new(|| Mutex::new(Images::default()));
fn decoded_image(bytes: &Arc<[u8]>) -> Result<Arc<ScanImage>, String> {
    let mut cache = IMAGES.lock().map_err(|_| "scan image cache is unavailable")?;
    if let Some(at) = cache.entries.iter().position(|(held, _)| Arc::ptr_eq(held, bytes)) {
        if let Some(entry) = cache.entries.remove(at) {
            let image = entry.1.clone();
            cache.entries.push_back(entry);
            return Ok(image);
        }
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes.as_ref())).with_guessed_format().map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|e| e.to_string())?.into_rgba8();
    let image = Arc::new(ScanImage { size: [decoded.width(), decoded.height()], rgba: Arc::new(decoded.into_raw()) });
    let charge = bytes.len().saturating_add(image.rgba.len());
    const BUDGET: usize = 512 * 1024 * 1024;
    while cache.bytes.saturating_add(charge) > BUDGET || cache.entries.len() >= 16 {
        let Some((encoded, old)) = cache.entries.pop_front() else { break };
        cache.bytes = cache.bytes.saturating_sub(encoded.len().saturating_add(old.rgba.len()));
    }
    if charge <= BUDGET {
        cache.bytes += charge;
        cache.entries.push_back((bytes.clone(), image.clone()));
    }
    Ok(image)
}
