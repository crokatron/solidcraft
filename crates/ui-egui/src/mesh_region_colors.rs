//! Partition indexed geometry by display color once. Navigation reuses these GPU batches.
use crate::{
    SolveApp,
    gpu::{GpuScene, InstanceLook, InstancedMesh, SHARED_SIZE, SharedTris},
};
use solidcraft_engine::{doc::Mat, geom::Mesh};
use std::sync::{Arc, mpsc};

pub(crate) struct RegionColors {
    pub body: String,
    pub mesh: Arc<Mesh>,
    pub selected: usize,
    ready: Option<mpsc::Receiver<Result<ColoredGroups, String>>>,
    groups: ColoredGroups,
    pub error: Option<String>,
}

type ColoredGroups = Vec<(u8, Arc<SharedTris>)>;

/// Keep unassigned facets neutral. Nearby IDs get separated hues, even where region sizes differ greatly.
pub(crate) fn color(code: u8) -> egui::Color32 {
    if code == 0 {
        return egui::Color32::from_gray(115);
    }
    let h = ((f32::from(code) - 1.) * 0.618034 + 0.56).fract();
    egui::ecolor::Hsva::new(h, 0.65 + 0.11 * f32::from(code % 2), 0.91, 1.).into()
}

impl RegionColors {
    pub fn new(app: &SolveApp, body: String, encoded: String) -> Option<Self> {
        let st = crate::open_meshing::session_world(&app.session);
        let (mesh, _) = st.body(&body)?.mesh_in_place();
        let shared = crate::viewport::cached_shared(app, &mesh);
        let source = mesh.clone();
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::Builder::new().name("mesh-region-colors".into()).spawn(move || {
            let tess = shared.unwrap_or_else(|| Arc::new(SharedTris::new(&source)));
            let groups = solidcraft_engine::doc::canvas::base64_decode(&encoded)
                .ok_or_else(|| "Invalid region palette".to_owned())
                .and_then(|labels| split_colors(&tess, &labels));
            let _ = tx.send(groups);
        });
        Some(Self { body, mesh, selected: 0, ready: Some(rx), groups: Vec::new(), error: handle.err().map(|e| e.to_string()) })
    }
    pub fn poll(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.ready else { return };
        match rx.try_recv() {
            Ok(Ok(groups)) => self.groups = groups,
            Ok(Err(err)) => self.error = Some(err),
            Err(mpsc::TryRecvError::Disconnected) => self.error = Some("Region coloring ended before completion".into()),
            Err(mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(40));
                return;
            }
        }
        self.ready = None;
        ctx.request_repaint();
    }
    pub fn generation(&self) -> usize {
        self.groups.as_ptr() as usize ^ self.groups.len()
    }
    pub fn count(&self) -> usize {
        self.groups.len()
    }
    pub fn highlight(&self, draws: &mut Vec<InstancedMesh>, place: &Mat, feedback: &mut GpuScene) {
        for (code, tris) in &self.groups {
            let [r, g, b, _] = color(*code).to_array();
            let color = feedback.color([r.saturating_add(44), g.saturating_add(44), b.saturating_add(44), 255]);
            draws.push(Self::placed(tris, place, color));
        }
    }
    fn placed(tris: &Arc<SharedTris>, place: &Mat, color: crate::gpu::PaletteIndex) -> InstancedMesh {
        let rows = [0, 1, 2].map(|r| [place[0][r] as f32, place[1][r] as f32, place[2][r] as f32, place[3][r] as f32]);
        let mut draw = InstancedMesh {
            key: Arc::as_ptr(tris) as u64,
            tris: tris.clone(),
            instances: Vec::new(),
            look: InstanceLook::Opaque,
            levels: Vec::new(),
            closed: false,
        };
        draw.place(rows, color);
        draw
    }
    pub fn draw(&self, sc: &mut GpuScene, mesh: &Arc<Mesh>, place: &Mat) -> bool {
        if self.groups.is_empty() || !Arc::ptr_eq(mesh, &self.mesh) {
            return false;
        }
        for (code, tris) in &self.groups {
            let palette = sc.color(color(*code).to_array());
            sc.instanced.push(Self::placed(tris, place, palette));
        }
        true
    }
}

fn split_colors(tess: &SharedTris, labels: &[u8]) -> Result<ColoredGroups, String> {
    let (triangles, _) = tess.indices.as_chunks::<3>();
    if triangles.len() != labels.len() {
        return Err(format!("The {} facet labels do not match the {} displayed triangles.", labels.len(), triangles.len()));
    }
    let mut buckets: [Vec<[u32; 3]>; 256] = std::array::from_fn(|_| Vec::new());
    for (&label, triangle) in labels.iter().zip(triangles) {
        buckets[label as usize].push(*triangle);
    }
    let mut map = vec![(u16::MAX, 0_u32); tess.vertices.len() / SHARED_SIZE];
    let mut groups = Vec::new();
    for (code, bucket) in buckets.into_iter().enumerate().filter(|(_, v)| !v.is_empty()) {
        let mut local = SharedTris { flat: tess.flat, bounds: tess.bounds, indices: Vec::with_capacity(bucket.len() * 3), ..Default::default() };
        for triangle in bucket {
            for vertex in triangle {
                let idx = vertex as usize;
                let entry = &mut map[idx];
                if entry.0 != code as u16 {
                    *entry = (code as u16, (local.vertices.len() / SHARED_SIZE) as u32);
                    local.vertices.extend_from_slice(&tess.vertices[idx * SHARED_SIZE..(idx + 1) * SHARED_SIZE]);
                }
                local.indices.push(entry.1);
            }
        }
        groups.push((code as u8, Arc::new(local)));
    }
    Ok(groups)
}

pub(crate) struct DenseLasso {
    pub token: usize,
    pub revision: u64,
    body: String,
    place: Mat,
    result: Option<mpsc::Receiver<Option<Arc<SharedTris>>>>,
    tris: Option<Arc<SharedTris>>,
}

impl DenseLasso {
    pub fn ready_for(&self, app: &SolveApp) -> bool {
        self.tris.is_some()
            && self.revision == app.session.revision
            && app
                .dialog
                .as_ref()
                .and_then(|d| d.facet_lasso.selection())
                .is_some_and(|(b, _, r, t)| self.body == b && self.revision == r && self.token == t)
    }
    pub fn draw(&self, scene: &mut GpuScene) {
        if let Some(tris) = &self.tris {
            let c = scene.color([255, 172, 28, 255]);
            scene.selected_instanced.push(RegionColors::placed(tris, &self.place, c));
        }
    }
}

/// Build the selected triangles only on a worker, when the user changes the lasso.
pub(crate) fn sync_lasso(app: &mut SolveApp, ctx: &egui::Context) {
    let Some((body, ranges, rev, token)) = app
        .dialog
        .as_ref()
        .filter(|d| !matches!(d.kind, crate::dialogs::Kind::MeshEdit { command: "pointcloud.copy_snippet", .. }))
        .and_then(|d| d.facet_lasso.selection())
        .filter(|(_, _, rev, _)| *rev == app.session.revision)
    else {
        app.dense_lasso = None;
        return;
    };
    if let Some(layer) = app.dense_lasso.as_mut().filter(|l| l.body == body && l.revision == rev && l.token == token) {
        if let Some(rx) = &layer.result {
            match rx.try_recv() {
                Ok(tris) => {
                    layer.tris = tris;
                    layer.result = None;
                    ctx.request_repaint();
                }
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(40)),
                Err(mpsc::TryRecvError::Disconnected) => layer.result = None,
            }
        }
        return;
    }
    let state = crate::open_meshing::session_world(&app.session);
    let Some(b) = state.body(body) else {
        app.dense_lasso = None;
        return;
    };
    let (mesh, place) = b.mesh_in_place();
    let shared = crate::viewport::cached_shared(app, &mesh);
    let session = app.session.scratch();
    let ranges = ranges.to_vec();
    let name = body.to_owned();
    let (tx, rx) = mpsc::channel();
    let _ = std::thread::Builder::new().name("mesh-lasso-colors".into()).spawn(move || {
        let result = session.scan_facet_display_indices(&name, &ranges).ok().and_then(|chosen| {
            let tess = shared.unwrap_or_else(|| Arc::new(SharedTris::new(&mesh)));
            let triangles = tess.indices.as_chunks::<3>().0;
            let mut local = SharedTris { flat: tess.flat, bounds: tess.bounds, indices: Vec::with_capacity(chosen.len() * 3), ..Default::default() };
            let mut remap = vec![u32::MAX; tess.vertices.len() / SHARED_SIZE];
            for i in chosen {
                for index in triangles.get(i as usize)? {
                    let k = *index as usize;
                    let local_id = remap.get_mut(k)?;
                    if *local_id == u32::MAX {
                        *local_id = (local.vertices.len() / SHARED_SIZE) as u32;
                        local.vertices.extend_from_slice(&tess.vertices[k * SHARED_SIZE..(k + 1) * SHARED_SIZE]);
                    }
                    local.indices.push(*local_id);
                }
            }
            Some(Arc::new(local))
        });
        let _ = tx.send(result);
    });
    app.dense_lasso = Some(DenseLasso { body: body.to_owned(), revision: rev, token, place, result: Some(rx), tris: None });
    ctx.request_repaint_after(std::time::Duration::from_millis(35));
}
