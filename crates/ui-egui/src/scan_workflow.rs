//! A lightweight command workspace: retain the scan source while changing tools.
use crate::{SolveApp, ui_kit};
use solidcraft_engine::Sel;
use solidcraft_ui_text::tr_label;

#[derive(Default)]
pub(crate) struct State {
    source: String,
    stage: usize,
}

// The sequence is guidance, never a claim that an operation has succeeded.
const STAGES: &[(&str, &str, &[(&str, &str)])] = &[
    (
        "1 · Preserve and clean",
        "Save a working copy, then isolate the part and remove unwanted scan data. Keep the original scan for comparison.",
        &[
            ("file.save_as", "Save working copy"),
            ("file.insert_mesh", "Insert scan"),
            ("mesh.isolate_facets", "Lasso isolate"),
            ("mesh.delete_facets", "Lasso delete"),
            ("mesh.trim_plane", "Plane trim"),
            ("mesh.remove_islands", "Remove small islands"),
            ("mesh.decimate", "Decimate scan"),
            ("mesh.global_remesh", "Global remesh"),
        ],
    ),
    (
        "2 · Establish datums",
        "Fit a reliable region to a plane. Align it to the coordinate system, then rotate and move within the aligned plane.",
        &[
            ("mesh.fit_region_plane", "Fit datum plane"),
            ("mesh.align_plane_to_csys", "Align plane to CSYS"),
            ("mesh.align_datum", "Align to datum"),
            ("mesh.align_landmarks", "Landmark alignment"),
        ],
    ),
    (
        "3 · Inspect useful regions",
        "Separate mounting details from the visible skin. Inspect sections before choosing the surfaces to rebuild.",
        &[
            ("mesh.auto_regions", "Detect regions"),
            ("mesh.copy_facet_cloud", "Copy selected patch"),
            ("mesh.copy_point_cloud", "Copy to point cloud"),
            ("scan.live_slice", "Inspect live slice"),
        ],
    ),
    (
        "4 · Create editable profiles",
        "Use thin scan slices (0.5-1 mm) and orient sections across the shape. Add closely spaced sections at tight bends. Trace or fit the wall profiles, then draw 3D guides through corresponding section endpoints.",
        &[
            ("scan.live_slice", "Position and trace slice"),
            ("mesh.auto_sections", "Create section profiles"),
            ("mesh.section", "Create section sketch"),
            ("sketch.create", "New sketch"),
            ("sketch.line3d", "3D guide line"),
            ("sketch.spline3d", "3D guide spline"),
        ],
    ),
    (
        "5 · Reconstruct surfaces",
        "Loft adjacent sections with the 3D guide curves. Add a local guide if the surface twists. Use boundary patches at tapered tips, then stitch and thicken. Rebuild mounting features separately.",
        &[
            ("mesh.fit_surface", "Fit scan surface"),
            ("surface.loft", "Loft profiles"),
            ("surface.boundary", "Boundary surface"),
            ("surface.patch", "Patch boundary"),
            ("surface.stitch", "Stitch surfaces"),
            ("surface.thicken", "Thicken surface"),
        ],
    ),
    (
        "6 · Check and deliver",
        "Inspect the result and save the editable design. Compare Scans currently compares two meshes; CAD-to-scan deviation is not available yet.",
        &[
            ("mesh.inspect_topology", "Inspect mesh"),
            ("mesh.compare_scans", "Compare two scans"),
            ("file.save_as", "Save design"),
            ("file.save_mesh", "Export mesh"),
        ],
    ),
];

pub(crate) fn show(app: &mut SolveApp, ctx: &egui::Context) {
    let Some(mut state) = app.scan_workflow.take() else { return };
    let sources: Vec<String> = app
        .session
        .world_state()
        .ok()
        .into_iter()
        .flat_map(|s| s.bodies.iter().filter(|b| b.body.is_mesh()).map(|b| b.name.clone()).collect::<Vec<_>>())
        .collect();
    if !sources.contains(&state.source) {
        state.source = app
            .session
            .selection
            .iter()
            .find_map(|s| match s {
                Sel::Body { name } | Sel::Face { body: name, .. } if sources.contains(name) => Some(name.clone()),
                _ => None,
            })
            .or_else(|| sources.first().cloned())
            .unwrap_or_default();
    }
    let mut command = None;
    let (closed, action) = ui_kit::Window::new(egui::Id::unique("scan_workflow"), "section", "Reverse Engineering")
        .placed(egui::Align2::LEFT_TOP, egui::pos2(280., 190.))
        .fields()
        .actions("", "Close", None)
        .show(ctx, true, |ui| {
            ui.label(tr_label("Scan → datums → profiles → surfaces → validation"));
            if sources.is_empty() {
                ui.label(tr_label("Insert a scan to begin."));
            } else {
                let mut index = sources.iter().position(|s| s == &state.source).unwrap_or(0);
                ui_kit::row(ui, "Source scan", |ui| {
                    ui_kit::dropdown(ui, "workflow_source", &mut index, &sources.iter().map(String::as_str).collect::<Vec<_>>())
                });
                if let Some(source) = sources.get(index) {
                    state.source.clone_from(source);
                }
            }
            for (index, (title, _, _)) in STAGES.iter().enumerate() {
                let response = ui.selectable_label(state.stage == index, tr_label(title));
                solidcraft_ui_registry::publish_handle(&format!("workflow:stage:{index}"), response.rect.center());
                if response.clicked() {
                    state.stage = index;
                }
            }
            ui.separator();
            if let Some((_, guidance, tools)) = STAGES.get(state.stage) {
                ui.add(egui::Label::new(tr_label(guidance)).wrap());
                for (id, label) in *tools {
                    let requires_scan = id.starts_with("mesh.") || *id == "scan.live_slice";
                    let response = ui.add_enabled(!requires_scan || !state.source.is_empty(), egui::Button::new(tr_label(label)));
                    solidcraft_ui_registry::publish_handle(&format!("workflow:{id}"), response.rect.center());
                    if response.clicked() {
                        command = Some(*id);
                    }
                }
            }
        });
    if closed || action != ui_kit::Action::None {
        return;
    }
    if let Some(id) = command {
        // Surface tools keep their sketch/edge selections; scan tools receive the retained source.
        if (id.starts_with("mesh.") || id == "scan.live_slice") && !state.source.is_empty() {
            app.session.selection = vec![Sel::Body { name: state.source.clone() }];
        }
        app.start(id);
    }
    app.scan_workflow = Some(state);
}
