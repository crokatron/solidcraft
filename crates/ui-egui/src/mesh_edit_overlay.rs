//! Highlight the individual scan vertex, edge or triangle a direct edit will affect.
use crate::SolveApp;
use crate::dialogs::Kind;
use crate::viewport::{Hit, Proj};
use egui::{Painter, Stroke};
use solidcore_record::Vec3;
use solidcraft_engine::{Sel, doc::apply_point, geom::prelude::*};
use solidcraft_theme::Tokens;
use std::sync::{Arc, Mutex};

type RegionMesh = Result<(usize, Vec<[Vec3; 3]>), String>;

#[derive(Clone, Debug)]
pub(crate) struct RegionPreview {
    seed: Sel,
    angle: String,
    revision: u64,
    result: Arc<Mutex<Option<RegionMesh>>>,
}

impl RegionPreview {
    pub(crate) fn counts(&self) -> Option<(usize, usize)> {
        let result = self.result.try_lock().ok()?;
        let (count, triangles) = result.as_ref()?.as_ref().ok()?;
        Some((*count, triangles.len()))
    }
}

pub(crate) fn region_controls(
    app: &SolveApp,
    ui: &mut egui::Ui,
    inputs: &[crate::selection::SelInput],
    angle: &str,
    preview: &mut Option<RegionPreview>,
) {
    let seed = inputs.first().and_then(|i| i.items.first());
    if preview.as_ref().is_some_and(|p| Some(&p.seed) != seed || p.angle != angle || p.revision != app.session.revision) {
        *preview = None;
    }
    if let Some(p) = preview {
        if let Ok(result) = p.result.lock() {
            match &*result {
                None => {
                    ui.label("Computing region…");
                }
                Some(Ok((n, _))) => {
                    ui.label(format!("{n} triangles in region"));
                }
                Some(Err(reason)) => {
                    ui.label(reason);
                }
            }
        }
        return;
    }
    let Some(seed @ Sel::Face { body, point, .. }) = seed else { return };
    let Ok(a) = app.session.doc.eval(angle, solidcraft_engine::doc::expr::Kind::Unitless) else { return };
    if !(0.0..=180.0).contains(&a) {
        return;
    }
    let response = ui.button("Preview Region");
    solidcraft_ui_registry::publish_handle("dialog:Preview Region", response.rect.center());
    if !response.clicked() {
        return;
    }
    let result = Arc::new(Mutex::new(None));
    *preview = Some(RegionPreview { seed: seed.clone(), revision: app.session.revision, angle: angle.to_owned(), result: result.clone() });
    let session = app.session.scratch();
    let params = serde_json::json!({"body":body,"point":[point.x,point.y,point.z],"angle":a});
    let ctx = ui.ctx().clone();
    let output = result.clone();
    if let Err(e) = std::thread::Builder::new().name("mesh-region-preview".into()).spawn(move || {
        let mesh = session.preview_mesh_region(&params).map_err(|e| e.to_string());
        if let Ok(mut data) = output.lock() {
            *data = Some(mesh);
        }
        ctx.request_repaint();
    }) && let Ok(mut data) = result.lock()
    {
        *data = Some(Err(e.to_string()));
    }
}

pub(crate) fn paint(app: &SolveApp, painter: &Painter, proj: &Proj) {
    crate::mesh_lasso::paint(app, painter, proj);
    if let Some(r) = &app.mesh_report
        && let (Some(samples), Some(tolerance)) = (r["overlay"].as_array(), r["tolerance_mm"].as_f64())
    {
        for sample in samples {
            let p = &sample["point"];
            if let (Some(x), Some(y), Some(z), Some(d)) = (p[0].as_f64(), p[1].as_f64(), p[2].as_f64(), sample["distance_mm"].as_f64())
                && let Some(screen) = proj.to_screen(Vec3::new(x, y, z))
            {
                painter.circle_filled(screen, 2.5, crate::mesh_report::sample_color(d <= tolerance));
            }
        }
    }
    let Some(dialog) = &app.dialog else { return };
    let Kind::MeshEdit { command, ref amount, .. } = dialog.kind else { return };
    crate::scan_alignment_ui::paint(command, &dialog.inputs, painter, proj);
    if let Some(p) = &dialog.mesh_region_preview
        && p.revision == app.session.revision
        && &p.angle == amount
        && dialog.inputs.first().and_then(|i| i.items.first()) == Some(&p.seed)
        && let Ok(data) = p.result.try_lock()
        && let Some(Ok((_, triangles))) = &*data
    {
        let color = Tokens::get().sel_edge.gamma_multiply(0.55);
        let mut patch = egui::Mesh::default();
        for tri in triangles {
            if let [Some(a), Some(b), Some(c)] = tri.map(|v| proj.to_screen(v)) {
                let at = patch.vertices.len() as u32;
                for p in [a, b, c] {
                    patch.colored_vertex(p, color);
                }
                patch.add_triangle(at, at + 1, at + 2);
            }
        }
        painter.add(egui::Shape::mesh(patch));
    }
    if !matches!(
        command,
        "mesh.move_vertex"
            | "mesh.split_edge"
            | "mesh.flip_edge"
            | "mesh.subdivide_face"
            | "mesh.delete_face"
            | "mesh.delete_island"
            | "mesh.delete_region"
            | "mesh.isolate_region"
            | "mesh.fit_region_plane"
            | "mesh.fit_surface"
            | "mesh.fit_region_cylinder"
    ) {
        return;
    }
    let shown = crate::open_meshing::shown_world(app);
    let draw = |name: &str, point: Vec3, selected: bool| -> Option<()> {
        let b = shown.body(name)?;
        if !b.body.is_mesh() {
            return None;
        }
        let (mesh, place) = b.mesh_in_place();
        let index = app.viewport.pick_index.of(&mesh);
        // Wait for the background spatial index on a large scan. A cosmetic overlay should
        // never send a second linear ray through millions of triangles on the UI thread.
        if index.is_none() && mesh.triangles.len() > 1024 {
            return None;
        }
        let screen = proj.to_screen(point)?;
        let (o, d) = proj.ray(screen);
        let (lo, ld) = crate::viewport::local_ray(&place, o, d)?;
        let (distance, i) = crate::pick_index::raycast(index.as_deref(), &mesh, lo, ld)?;
        if (o + d * distance).dist(point) > 1e-4 * (1. + distance.abs().sqrt()) {
            return None;
        }
        let local = mesh.tri(mesh.triangles.get(i)?)?;
        let world = local.map(|p| apply_point(&place, p));
        let [a, b, c] = world.map(|p| proj.to_screen(p));
        let projected = [a?, b?, c?];
        let tokens = Tokens::get();
        let color = if selected { tokens.sel_edge } else { tokens.hover_edge };
        let stroke = Stroke::new(if selected { 3. } else { 2. }, color);
        let nearest = |edge: bool| {
            (0..3)
                .min_by(|&i, &j| {
                    let dist = |k: usize| if edge { point.dist_to_segment(world[k], world[(k + 1) % 3]) } else { point.dist(world[k]) };
                    dist(i).total_cmp(&dist(j))
                })
                .unwrap_or(0)
        };
        match command {
            "mesh.move_vertex" => {
                painter.circle_filled(projected[nearest(false)], 4., color);
            }
            "mesh.split_edge" | "mesh.flip_edge" => {
                let j = nearest(true);
                painter.line_segment([projected[j], projected[(j + 1) % 3]], stroke);
            }
            _ => {
                for k in 0..3 {
                    painter.line_segment([projected[k], projected[(k + 1) % 3]], stroke);
                }
            }
        }
        Some(())
    };
    if let Some(Hit::Face { body, point, .. }) = &app.viewport.hover {
        draw(body, *point, false);
    }
    if let Some(Sel::Face { body, point, .. }) = dialog.inputs.first().and_then(|s| s.items.first()) {
        draw(body, *point, true);
    }
}
