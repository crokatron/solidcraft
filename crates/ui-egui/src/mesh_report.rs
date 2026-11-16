//! The topology counts from the completed Inspect Mesh operation.
use crate::{SolveApp, ui_kit};
use serde_json::Value;

pub(crate) fn has_report(id: &str) -> bool {
    matches!(
        id,
        "mesh.inspect_topology"
            | "surface.match"
            | "mesh.heal_scan"
            | "mesh.fit_region_plane"
            | "mesh.fit_surface"
            | "mesh.fit_surface.refit"
            | "mesh.fit_region_cylinder"
            | "mesh.symmetry_plane"
            | "mesh.align_plane_to_csys"
            | "mesh.align_landmarks"
            | "mesh.align_datum"
            | "mesh.align_coarse"
            | "mesh.align_scans"
            | "mesh.compare_scans"
            | "mesh.auto_regions"
            | "mesh.copy_point_cloud"
            | "mesh.copy_facet_cloud"
            | "pointcloud.copy_snippet"
            | "mesh.auto_sections"
            | "mesh.global_remesh"
            | "mesh.decimate"
    )
}

pub(crate) fn record(app: &mut SolveApp, id: &str, reply: &Value) {
    let mut reply = reply.clone();
    app.mesh_region_colors = None;
    if id == "mesh.auto_regions"
        && let Some(rows) = reply.as_object_mut()
    {
        rows.remove("overlay");
        if let (Some(Value::String(encoded)), Some(name)) = (rows.remove("palette_indices_b64"), rows.get("body").and_then(Value::as_str)) {
            app.mesh_region_colors = crate::mesh_region_colors::RegionColors::new(app, name.to_owned(), encoded);
        }
    }
    if id != "mesh.inspect_topology" {
        reply["report_command"] = id.into();
    }
    app.mesh_report = Some(reply);
}

pub(crate) fn window_command(report: &Value) -> &'static str {
    match report["report_command"].as_str() {
        Some("surface.match") => "surface.match",
        Some("mesh.copy_point_cloud") => "mesh.copy_point_cloud",
        Some("mesh.copy_facet_cloud") => "mesh.copy_facet_cloud",
        Some("pointcloud.copy_snippet") => "pointcloud.copy_snippet",
        Some("mesh.auto_sections") => "mesh.auto_sections",
        Some("mesh.auto_regions") => "mesh.auto_regions",
        Some("mesh.global_remesh") => "mesh.global_remesh",
        Some("mesh.decimate") => "mesh.decimate",
        Some("mesh.fit_surface") => "mesh.fit_surface",
        Some("mesh.fit_surface.refit") => "mesh.fit_surface.refit",
        Some("mesh.fit_region_cylinder") => "mesh.fit_region_cylinder",
        Some("mesh.fit_region_plane") => "mesh.fit_region_plane",
        Some("mesh.symmetry_plane") => "mesh.symmetry_plane",
        Some("mesh.align_plane_to_csys") => "mesh.align_plane_to_csys",
        Some("mesh.align_landmarks") => "mesh.align_landmarks",
        Some("mesh.align_datum") => "mesh.align_datum",
        Some("mesh.align_coarse") => "mesh.align_coarse",
        Some("mesh.align_scans") => "mesh.align_scans",
        Some("mesh.compare_scans") => "mesh.compare_scans",
        Some("mesh.heal_scan") => "mesh.heal_scan",
        _ => "mesh.inspect_topology",
    }
}

pub(crate) fn show(app: &mut SolveApp, ctx: &egui::Context) {
    if let Some(colors) = &mut app.mesh_region_colors {
        colors.poll(ctx);
    }
    let Some(report) = &app.mesh_report else { return };
    let command = window_command(report);
    let summary = ["match_report", "heal_report", "fit_report", "align_report", "remesh_report", "decimate_report", "copy_report"]
        .iter()
        .find_map(|k| report[*k].as_str());
    let title = solidcraft_ui_text::tr_label(match command {
        "surface.match" => "ui.match_surface_title",
        "mesh.fit_surface" => "Fit Scan Surface",
        "mesh.fit_surface.refit" => "Refit Scan Surface",
        "mesh.fit_region_cylinder" => "Fit Region Cylinder",
        "mesh.fit_region_plane" => "Fit Region Plane",
        "mesh.symmetry_plane" => "cmd.mesh.symmetry_plane",
        "mesh.align_plane_to_csys" => "Align Plane to CSYS",
        "mesh.align_landmarks" => "Landmark Align",
        "mesh.align_datum" => "Align to Datum",
        "mesh.align_coarse" => "Auto Align",
        "mesh.align_scans" => "Fine Align Scans",
        "mesh.compare_scans" => "Compare Scans",
        "mesh.heal_scan" => "Heal Scan",
        "mesh.copy_point_cloud" => "Copy to Point Cloud",
        "mesh.copy_facet_cloud" => "Copy Lasso Patch",
        "pointcloud.copy_snippet" => "Copy Cloud Snippet",
        "mesh.auto_sections" => "Auto Sections",
        "mesh.auto_regions" => "Auto Regions",
        "mesh.global_remesh" => "Global Remesh",
        "mesh.decimate" => "Decimate",
        _ => "Mesh Topology",
    });
    let mut use_region = None;
    let mut align_plane = None;
    let mut refit_surface = None;
    let (closed, action) = ui_kit::Window::new(egui::Id::unique("mesh_topology_report"), "inspect", title)
        .fields()
        .actions("", solidcraft_ui_text::tr("ui.close"), None)
        .show(ctx, true, |ui| {
            if let Some(body) = report["body"].as_str() {
                ui_kit::row(ui, solidcraft_ui_text::tr_label("Mesh body"), |ui| ui_kit::text(ui, body, ui_kit::Text::Cell));
            }
            if matches!(command, "mesh.fit_surface" | "mesh.fit_surface.refit") {
                crate::scan_surface_fit::report(ui, report);
                if let Some(feature) = report["feature"].as_u64() {
                    let response = ui.button(solidcraft_ui_text::tr("ui.scan_fit.refit"));
                    solidcraft_ui_registry::publish_handle("fit:refit_surface", response.rect.center());
                    if response.clicked() {
                        refit_surface = Some(feature);
                    }
                }
                return;
            }
            if command == "surface.match" {
                if let Some(summary) = summary {
                    ui.add(egui::Label::new(summary).wrap());
                }
                let metrics = &report["continuity"];
                for (key, label) in [
                    ("max_gap_mm", "ui.surface_match.gap"),
                    ("max_normal_angle_deg", "ui.surface_match.angle"),
                    ("max_curvature_jump_per_mm", "ui.surface_match.curvature"),
                    ("seam_gap_bound_mm", "ui.surface_match.gap_bound"),
                    ("source_displacement_bound_mm", "ui.surface_match.move_bound"),
                    ("control_points_u", "ui.surface_match.cv_u"),
                    ("control_points_v", "ui.surface_match.cv_v"),
                ] {
                    if let Some(value) = metrics[key].as_f64() {
                        ui.label(format!("{}: {value:.6}", solidcraft_ui_text::tr(label)));
                    }
                }
                ui.add(egui::Label::new(solidcraft_ui_text::tr("ui.surface_match.metrics_help")).wrap());
                return;
            }
            if let Some(summary) = summary {
                ui.add(egui::Label::new(summary).wrap());
                if matches!(command, "mesh.fit_region_plane" | "mesh.symmetry_plane")
                    && let Some(id) = report["feature"].as_u64()
                {
                    let response = ui.button("Align Plane to CSYS");
                    solidcraft_ui_registry::publish_handle("fit:align_plane", response.rect.center());
                    if response.clicked() {
                        align_plane = Some(id);
                    }
                }
                return;
            }
            if command == "mesh.auto_sections" {
                if let Some(msg) = report["sections_report"].as_str() {
                    ui.label(msg);
                }
                if let Some(rows) = report["profiles"].as_array() {
                    egui::ScrollArea::vertical().max_height(160.).show(ui, |ui| {
                        for row in rows {
                            ui.label(format!(
                                "{} · {:.3} mm · {} closed profiles",
                                row["name"].as_str().unwrap_or("Sketch"),
                                row["offset"].as_f64().unwrap_or(0.),
                                row["closed_profiles"].as_u64().unwrap_or(0)
                            ));
                        }
                    });
                }
                return;
            }
            if command == "mesh.auto_regions" {
                if let Some(text) = report["regions_report"].as_str() {
                    ui.add(egui::Label::new(text).wrap());
                }
                if let Some(colors) = &mut app.mesh_region_colors {
                    if let Some(err) = &colors.error {
                        ui.label(err);
                    } else if colors.count() == 0 {
                        ui.label("Preparing surface colors…");
                    }
                    if let Some(rows) = report["regions"].as_array().filter(|s| !s.is_empty()) {
                        egui::ScrollArea::vertical().max_height(140.).show_rows(ui, 19., rows.len(), |ui, range| {
                            for i in range {
                                let row = &rows[i];
                                let label = format!("Region {} · {} triangles", i + 1, row["triangles"].as_u64().unwrap_or(0));
                                let code = (1 + i % 255) as u8;
                                ui.horizontal(|ui| {
                                    ui.colored_label(crate::mesh_region_colors::color(code), "■");
                                    let b = ui.selectable_label(colors.selected == i, label);
                                    solidcraft_ui_registry::publish_handle(&format!("region:{}", i + 1), b.rect.center());
                                    if b.clicked() {
                                        colors.selected = i;
                                    }
                                });
                            }
                        });
                        let i = colors.selected.min(rows.len() - 1);
                        for (label, id) in [
                            ("Isolate Region", "mesh.isolate_region"),
                            ("Delete Region", "mesh.delete_region"),
                            ("Fit Plane", "mesh.fit_region_plane"),
                            ("Fit Scan Surface", "mesh.fit_surface"),
                        ] {
                            let b = ui.button(solidcraft_ui_text::tr_label(label));
                            solidcraft_ui_registry::publish_handle(&format!("region:{id}"), b.rect.center());
                            if b.clicked() {
                                use_region = Some((id, rows[i].clone(), colors.body.clone(), report["angle"].as_f64().unwrap_or(25.)));
                            }
                        }
                    }
                }
                return;
            }
            if command == "mesh.compare_scans" {
                if let Some(reference) = report["reference"].as_str() {
                    ui_kit::row(ui, "Reference scan", |ui| ui_kit::text(ui, reference, ui_kit::Text::Cell));
                }
                for (label, key, unit) in [
                    ("Mean distance", "mean_mm", " mm"),
                    ("RMS distance", "rms_mm", " mm"),
                    ("Maximum distance", "max_mm", " mm"),
                    ("Within tolerance", "coverage_percent", "%"),
                    ("Tolerance", "tolerance_mm", " mm"),
                ] {
                    if let Some(v) = report[key].as_f64() {
                        ui_kit::row(ui, label, |ui| ui_kit::text(ui, &format!("{v:.4}{unit}"), ui_kit::Text::Cell));
                    }
                }
                for (label, within) in [("Within tolerance", true), ("Over tolerance", false)] {
                    ui.colored_label(sample_color(within), format!("●  {label}"));
                }
                return;
            }
            for (label, key) in [
                ("Triangles", "triangles"),
                ("Boundary edges", "boundary_edges"),
                ("Nonmanifold edges", "nonmanifold_edges"),
                ("Winding conflicts", "winding_conflicts"),
                ("Degenerate triangles", "degenerate_triangles"),
            ] {
                let count = report[key].as_u64().map(|n| n.to_string()).unwrap_or_else(|| "—".into());
                ui_kit::row(ui, solidcraft_ui_text::tr_label(label), |ui| ui_kit::text(ui, &count, ui_kit::Text::Cell));
            }
        });
    if closed || action != ui_kit::Action::None || use_region.is_some() || align_plane.is_some() || refit_surface.is_some() {
        app.mesh_report = None;
        app.mesh_region_colors = None;
    }
    if let Some(id) = refit_surface {
        crate::scan_surface_fit::start_refit(app, id);
    }
    if let Some(id) = align_plane {
        crate::scan_alignment_ui::start_plane(app, id);
    }
    if let Some((id, row, body, angle)) = use_region
        && let Ok([x, y, z]) = serde_json::from_value::<[f64; 3]>(row["seed_point"].clone())
    {
        app.start(id);
        if let Some(d) = &mut app.dialog
            && let crate::dialogs::Kind::MeshEdit { amount, .. } = &mut d.kind
        {
            *amount = angle.to_string();
            d.inputs[0].items = vec![solidcraft_engine::Sel::Face { body, index: 0, point: solidcore_record::Vec3::new(x, y, z) }];
            d.extra.insert("auto_region".into(), row["region"].clone());
            if id == "mesh.fit_surface" {
                d.scan_surface_fit.selection = 0;
            }
        }
    }
}

pub(crate) fn sample_color(within: bool) -> egui::Color32 {
    if within { egui::Color32::from_rgb(76, 202, 169) } else { egui::Color32::from_rgb(247, 136, 64) }
}
