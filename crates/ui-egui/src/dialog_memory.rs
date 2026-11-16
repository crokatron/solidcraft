//! Versioned, explicit reusable dialog settings. Geometry and document references never enter this record.
use crate::{
    SolveApp,
    dialogs::{Dialog, Kind},
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Map, Value, json};
use solidcraft_engine::doc::{Document, expr::Kind as Unit};

struct Fields<'a> {
    source: Option<&'a Value>,
    values: Map<String, Value>,
    doc: &'a Document,
}
impl Fields<'_> {
    fn field<T: Serialize + DeserializeOwned>(&mut self, key: &str, value: &mut T) {
        if let Some(source) = self.source {
            if let Some(v) = source.get(key).filter(|v| bounded(v, 0))
                && let Ok(next) = serde_json::from_value(v.clone())
            {
                *value = next;
            }
        } else if let Ok(mut v) = serde_json::to_value(value) {
            freeze(self.doc, key, &mut v);
            if bounded(&v, 0) {
                self.values.insert(key.into(), v);
            }
        }
    }
    fn index(&mut self, key: &str, value: &mut usize, max: usize) {
        if let Some(source) = self.source {
            if let Some(v) = source.get(key).and_then(Value::as_u64).filter(|v| *v <= max as u64) {
                *value = v as usize;
            }
        } else {
            self.values.insert(key.into(), json!(value));
        }
    }
}
fn bounded(v: &Value, depth: usize) -> bool {
    if depth > 6 {
        return false;
    }
    match v {
        Value::String(s) => s.len() <= 4096,
        Value::Array(a) => a.len() <= 128 && a.iter().all(|v| bounded(v, depth + 1)),
        Value::Object(o) => o.len() <= 128 && o.values().all(|v| bounded(v, depth + 1)),
        _ => true,
    }
}
fn freeze(doc: &Document, key: &str, v: &mut Value) {
    match v {
        Value::String(s) if !s.trim().is_empty() => {
            let unit = if key.contains("angle") || key.ends_with("taper") || key.ends_with("twist") || ["rx", "ry", "rz"].contains(&key) {
                Unit::Angle
            } else {
                Unit::Unitless
            };
            let detached = Document::default();
            // Unitful literals remain portable and preserve the user's spelling.
            if detached.eval(s, unit).is_ok() || detached.eval(s, Unit::Length).is_ok() {
                return;
            }
            if let Ok(number) = doc.eval(s, unit) {
                *s = if unit == Unit::Angle { format!("{} deg", number.to_degrees()) } else { number.to_string() };
            } else if let Ok(number) = doc.eval(s, Unit::Length) {
                *s = format!("{number} mm");
            } else if ![
                "designation",
                "thread",
                "hole.thread",
                "form",
                "hand",
                "hole.hand",
                "tip",
                "steep_flank",
                "continuity",
                "alignment",
                "alignment2",
                "type",
                "direction",
                "fit",
                "corner",
                "section",
                "section_position",
                "from_snap",
                "to_snap",
                "convention",
            ]
            .contains(&key)
            {
                *v = Value::Null;
            }
        }
        Value::Array(a) => {
            for v in a {
                freeze(doc, key, v);
            }
        }
        Value::Object(o) => {
            for (k, v) in o {
                freeze(doc, k, v);
            }
        }
        _ => {}
    }
}
macro_rules! fields {
    ($f:ident; $($v:ident),* $(,)?) => {$( $f.field(stringify!($v),$v); )*};
}
macro_rules! indexes {
    ($f:ident; $($v:ident:$max:expr),* $(,)?) => {$( $f.index(stringify!($v),$v,$max); )*};
}
fn kind_fields(kind: &mut Kind, f: &mut Fields<'_>) {
    use crate::{dialogs_plastic::Pl, dialogs_sheet::Sm, dialogs_surface::Surf, sketch_modify_ui::Mod};
    match kind {
        Kind::Shell { thickness, .. } => {
            fields!(f;thickness);
        }
        Kind::Draft { angle, .. } => {
            fields!(f;angle);
        }
        Kind::OffsetPlane { offset, .. } => {
            fields!(f;offset);
        }
        Kind::AnglePlane { angle, direction_only, .. } => {
            fields!(f;angle,direction_only);
        }
        Kind::TangentPlane { angle, .. } => {
            fields!(f;angle);
        }
        Kind::Scale { factor, per_axis, factor_y, factor_z, .. } => {
            fields!(f;factor,per_axis,factor_y,factor_z);
        }
        Kind::OffsetFaces { distance, .. } => {
            fields!(f;distance);
        }
        Kind::Stock { margin, .. } => {
            fields!(f;margin);
        }
        Kind::Pipe { diameter, wall, operation, .. } => {
            fields!(f;diameter,wall);
            indexes!(f;operation:3);
        }
        Kind::Emboss { depth, deboss, .. } => {
            fields!(f;depth,deboss);
        }
        Kind::Extrude {
            distance,
            auto_op,
            distance2,
            taper,
            taper_present,
            to_offset,
            start,
            all,
            thin,
            wall,
            wall2,
            direction,
            operation,
            wall_side,
            ..
        } => {
            fields!(f;distance,auto_op,distance2,taper,taper_present,to_offset,start,all,thin,wall,wall2);
            indexes!(f;direction:4,operation:3,wall_side:3);
        }
        Kind::Revolve { angle, operation, .. } => {
            fields!(f;angle);
            indexes!(f;operation:3);
        }
        Kind::Fillet { radius, chamfer, chain, distance2, angle, flip, setback, ctype, .. } => {
            fields!(f;radius,chamfer,chain,distance2,angle,flip,setback);
            indexes!(f;ctype:2);
        }
        Kind::PatternRect { count, spacing, count2, spacing2, .. } => {
            fields!(f;count,spacing,count2,spacing2);
        }
        Kind::PatternCirc { count, angle, .. } => {
            fields!(f;count,angle);
        }
        Kind::Loft { operation, rail_fit, .. } => {
            indexes!(f;operation:3,rail_fit:1);
        }
        Kind::MeshEdit { delta, amount, plane, side, .. } => {
            fields!(f;delta,amount);
            indexes!(f;plane:2,side:1);
        }
        Kind::MeshConvert { tolerance, .. } => {
            fields!(f;tolerance);
        }
        Kind::Construct { t, .. } => {
            fields!(f;t);
        }
        Kind::PathPlane { t, .. } => {
            fields!(f;t);
        }
        Kind::Rib { web, thickness, depth, flip, normal, .. } => {
            fields!(f;web,thickness,depth,flip,normal);
        }
        Kind::BoundaryFill { operation, .. } => {
            indexes!(f;operation:3);
        }
        Kind::Align { flip, components, axes, angle, .. } => {
            fields!(f;flip,components,axes,angle);
        }
        Kind::PathPattern { count, spacing, .. } => {
            fields!(f;count,spacing);
        }
        Kind::Section { offset, flip, hatch, .. } => {
            fields!(f;offset,flip,hatch);
        }
        Kind::Move { x, y, z, rx, ry, rz, angle, mode, .. } => {
            fields!(f;x,y,z,rx,ry,rz,angle);
            indexes!(f;mode:4);
        }
        Kind::Combine { keep_tools, operation, .. } => {
            fields!(f;keep_tools);
            indexes!(f;operation:3);
        }
        Kind::ViewDxf { dashed, tolerance, .. } => {
            fields!(f;dashed,tolerance);
        }
        Kind::SketchIntersect { bodies, projection_link, .. } => {
            fields!(f;bodies,projection_link);
        }
        Kind::SketchProject { bodies, projection_link, .. } => {
            fields!(f;bodies,projection_link);
        }
        Kind::Sweep { operation, opts } => {
            f.index("operation", operation, 3);
            sweep(opts, f);
        }
        Kind::Thread { designation, length, options, suggested, .. } => {
            if f.source.is_none() && suggested.as_deref() == Some(designation.as_str()) {
                designation.clear();
            }
            fields!(f;designation,length);
            let o = options;
            f.field("form", &mut o.form);
            f.field("hand", &mut o.hand);
            f.field("starts", &mut o.starts);
            f.index("extent", &mut o.extent, 2);
            f.field("tip", &mut o.tip);
            f.field("chamfer_angle", &mut o.chamfer_angle);
            f.field("chamfer_depth", &mut o.chamfer_depth);
            f.field("steep_flank", &mut o.steep_flank);
            ease(&mut o.ease_out, f);
        }
        Kind::Hole { diameter, depth, kind, cb_diameter, cb_depth, cs_diameter, cs_angle, opts } => {
            fields!(f;diameter,depth,cb_diameter,cb_depth,cs_diameter,cs_angle);
            f.index("kind", kind, 2);
            let o = opts;
            f.field("hole.multiple", &mut o.multiple);
            f.field("hole.all", &mut o.all);
            f.field("hole.angled", &mut o.angled);
            f.field("hole.tip_angle", &mut o.tip_angle);
            f.field("hole.tapped", &mut o.tapped);
            f.field("hole.thread", &mut o.thread);
            f.field("hole.thread_full_depth", &mut o.thread_full_depth);
            f.field("hole.thread_depth", &mut o.thread_depth);
            f.field("hole.hand", &mut o.hand);
            f.field("hole.modeled", &mut o.modeled);
            f.field("hole.drill", &mut o.drill);
            f.field("hole.bridge", &mut o.bridge);
            ease(&mut o.ease_out, f);
        }
        Kind::Primitive { fields, operation, .. } => {
            f.index("operation", operation, 3);
            for (key, value) in fields {
                f.field(key, value);
            }
        }
        Kind::EndCap(o) => {
            f.field("thickness", &mut o.thickness);
            f.field("use_offset", &mut o.use_offset);
            f.field("ratio", &mut o.ratio);
            f.field("offset", &mut o.offset);
            f.field("chamfer", &mut o.chamfer);
            f.index("direction", &mut o.direction, 1);
            f.index("inset_kind", &mut o.inset_kind, 1);
        }
        Kind::Frame(o) => {
            f.field("all", &mut o.all);
            f.index("kind", &mut o.kind, solidcraft_engine::doc::frame::profile_kinds().len().saturating_sub(1));
            f.field("dims", &mut o.dims);
            f.index("anchor", &mut o.anchor, 10);
            f.field("angle", &mut o.angle);
            f.field("mirror", &mut o.mirror);
        }
        Kind::Surface(surface) => match surface {
            Surf::Offset { distance, chain, .. } => {
                fields!(f;distance,chain);
            }
            Surf::Stitch { tolerance, .. } => {
                fields!(f;tolerance);
            }
            Surf::Thicken { thickness, symmetric, operation, .. } => {
                fields!(f;thickness,symmetric);
                indexes!(f;operation:3);
            }
            Surf::Extend { distance, .. } => {
                fields!(f;distance);
            }
            Surf::Fair { strength, iterations, keep_edges, tolerance, .. } => {
                fields!(f;strength,iterations,keep_edges,tolerance);
            }
            Surf::Match { order, cross_scale, reverse_target } => {
                fields!(f;cross_scale,reverse_target);
                indexes!(f;order:3);
            }
            Surf::Revolve { angle, symmetric, .. } => {
                fields!(f;angle,symmetric);
            }
            Surf::Loft { rail_fit, .. } => {
                indexes!(f;rail_fit:1);
            }
            Surf::Blend { continuity, bulge, stitch, .. } => {
                fields!(f;continuity,bulge,stitch);
            }
            Surf::Boundary { continuity, join, .. } => {
                fields!(f;continuity,join);
            }
            Surf::Patch { continuity, .. } => {
                fields!(f;continuity);
            }
            Surf::Ruled { distance, angle, flip, round, vector, mode, .. } => {
                fields!(f;distance,angle,flip,round,vector);
                indexes!(f;mode:2);
            }
            Surf::Trim { extend, stitch, .. } => {
                fields!(f;extend,stitch);
            }
            Surf::Untrim { outer, in_place, holes, .. } => {
                fields!(f;outer,in_place);
                indexes!(f;holes:2);
            }
            Surf::Extrude { distance, direction, edge } => {
                fields!(f;distance);
                f.index("direction", direction, 2);
                f.index("edge.method", &mut edge.method, 2);
                f.field("edge.vector", &mut edge.vector);
                f.field("edge.flip", &mut edge.flip);
            }
            Surf::Sweep(o) => sweep(o, f),
            _ => {}
        },
        Kind::Sheet(sheet) => match sheet {
            Sm::Perforate { diameter, pitch, margin, pattern, .. } => {
                fields!(f;diameter,pitch,margin);
                indexes!(f;pattern:1);
            }
            Sm::Hem { length, gap, flip, .. } => {
                fields!(f;length,gap,flip);
            }
            Sm::Fold { angle, radius, flip, position, .. } => {
                fields!(f;angle,radius,flip);
                indexes!(f;position:3);
            }
            Sm::Path { radius, .. } => {
                fields!(f;radius);
            }
            Sm::Unfold { refold, .. } => {
                fields!(f;refold);
            }
            Sm::Flange { height, angle, radius, flip, distance, reverse, relief_size, then, position, height_from, relief, .. } => {
                fields!(f;height,angle,radius,flip,distance,reverse,relief_size,then);
                indexes!(f;position:4,height_from:2,relief:4);
            }
            _ => {}
        },
        Kind::Plastic(plastic) => match plastic {
            Pl::Boss { diameter, height, hole_diameter, hole_depth, draft, fillet, ribs, rib_thickness, rib_length, rib_offset, .. } => {
                fields!(f;diameter,height,hole_diameter,hole_depth,draft,fillet,ribs,rib_thickness,rib_length,rib_offset);
            }
            Pl::Lip { groove, width, height, gap, outside, .. } => {
                fields!(f;groove,width,height,gap,outside);
            }
            Pl::Rest { round, width, length, height, draft, thickness, .. } => {
                fields!(f;round,width,length,height,draft,thickness);
            }
            Pl::SnapFit { length, thickness, width, catch_depth, catch_length, base_height, new_body, .. } => {
                fields!(f;length,thickness,width,catch_depth,catch_length,base_height,new_body);
            }
            _ => {}
        },
        Kind::SketchModify(modify) => match modify {
            Mod::Scale { factor, .. } => {
                fields!(f;factor);
            }
            Mod::Offset { distance, distance2, instances, chain, flip, mode, .. } => {
                fields!(f;distance,distance2,instances,chain,flip);
                indexes!(f;mode:2);
            }
            Mod::Move { x, y, angle, copy, .. } => {
                fields!(f;x,y,angle,copy);
            }
            Mod::Transform { x, y, factor, angle, .. } => {
                fields!(f;x,y,factor,angle);
            }
            Mod::PatternRect { count, spacing, count2, spacing2, alignment, alignment2, reversed, reversed2, .. } => {
                fields!(f;count,spacing,count2,spacing2,alignment,alignment2,reversed,reversed2);
            }
            Mod::PatternCirc { count, angle, alignment, reversed, .. } => {
                fields!(f;count,angle,alignment,reversed);
            }
            Mod::Fillet3d { radius, .. } => {
                fields!(f;radius);
            }
            Mod::Corner { chamfer, size, distance2, angle, flip, ctype, .. } => {
                fields!(f;chamfer,size,distance2,angle,flip);
                indexes!(f;ctype:2);
            }
            Mod::Equation { start, end, tolerance, range_unit, form, .. } => {
                fields!(f;start,end,tolerance,range_unit);
                indexes!(f;form:1);
            }
            Mod::Divide { count, .. } => {
                fields!(f;count);
            }
            _ => {}
        },
        _ => {}
    }
}
fn sweep(o: &mut crate::dialogs_sweep::SweepOpts, f: &mut Fields<'_>) {
    f.index("orientation", &mut o.orientation, 2);
    f.field("twist", &mut o.twist);
    f.field("scale", &mut o.scale);
    f.index("rail_fit", &mut o.rail_fit, 1);
}
fn ease(o: &mut crate::dialogs_thread::Ease, f: &mut Fields<'_>) {
    f.field("ease.fade", &mut o.fade);
    f.field("ease.length", &mut o.length);
    f.field("ease.short", &mut o.short);
}
pub(super) fn capture(app: &SolveApp, dialog: &Dialog) -> Value {
    let mut kind = dialog.kind.clone();
    for value in crate::length_input::fields(&app.session.doc, &mut kind) {
        if !value.trim().is_empty()
            && let Ok(mm) = crate::length_input::eval(&app.session.doc, value, Unit::Length)
        {
            *value = format!("{mm} mm");
        }
    }
    // Additional lengths owned by compound forms are not all in length_input's UI conversion list.
    let length = |s: &mut String| {
        if !s.trim().is_empty()
            && let Ok(mm) = crate::length_input::eval(&app.session.doc, s, Unit::Length)
        {
            *s = format!("{mm} mm");
        }
    };
    match &mut kind {
        Kind::MeshEdit { command, delta, amount, .. } => {
            for v in delta {
                length(v);
            }
            if matches!(*command, "mesh.global_remesh" | "mesh.compare_scans" | "mesh.trim_plane" | "mesh.section") {
                length(amount);
            }
        }
        Kind::EndCap(o) => {
            length(&mut o.thickness);
            length(&mut o.offset);
            length(&mut o.chamfer);
        }
        Kind::Frame(o) => {
            for v in o.dims.iter_mut().flatten() {
                length(v);
            }
        }
        Kind::Thread { options, .. } => {
            length(&mut options.chamfer_depth);
            length(&mut options.ease_out.length);
        }
        Kind::Hole { opts, .. } => {
            length(&mut opts.thread_depth);
            length(&mut opts.ease_out.length);
            if let Some(s) = &mut opts.drill {
                length(s);
            }
        }
        Kind::Sheet(crate::dialogs_sheet::Sm::Flange { then, .. }) => {
            for (_, height, _) in then {
                length(height);
            }
        }
        Kind::Sheet(crate::dialogs_sheet::Sm::Path { radius }) => length(radius),
        Kind::Sheet(crate::dialogs_sheet::Sm::Perforate { diameter, pitch, margin, .. }) => {
            length(diameter);
            length(pitch);
            length(margin);
        }
        _ => {}
    }
    let mut fields = Fields { source: None, values: Map::new(), doc: &app.session.doc };
    kind_fields(&mut kind, &mut fields);
    let mut extra = Map::new();
    for key in super::remembered_extra_keys(&kind) {
        if let Some(v) = dialog.extra.get(*key) {
            let mut v = v.clone();
            freeze(&app.session.doc, key, &mut v);
            if bounded(&v, 0) {
                extra.insert((*key).into(), v);
            }
        }
    }
    let (through, mode) = dialog.facet_lasso.options();
    let scan = dialog.scan_copy.memory(&app.session.doc);
    let scan_fit = dialog.scan_surface_fit.memory(&app.session.doc);
    json!({"version":1,"confirmation":dialog.last_confirmation,"operation_manual":dialog.op_auto.manual,"to_object":dialog.inputs.iter().any(|i|i.label=="To Object"),"fields":fields.values,"extra":extra,"lasso":{"through":through,"mode":mode},"scan":scan,"scan_fit":scan_fit})
}
pub(super) fn restore(app: &SolveApp, dialog: &mut Dialog, saved: &Value) {
    if saved.get("version").and_then(Value::as_u64) != Some(1) {
        return;
    }
    if let Some(word) = saved["confirmation"].as_str().filter(|word| matches!(*word, "ok" | "apply" | "cancel")) {
        dialog.last_confirmation = word.into();
    }
    dialog.op_auto.manual = saved.get("operation_manual").and_then(Value::as_bool).unwrap_or(false);
    if let Some(source) = saved.get("fields").filter(|v| bounded(v, 0)) {
        let mut fields = Fields { source: Some(source), values: Map::new(), doc: &app.session.doc };
        kind_fields(&mut dialog.kind, &mut fields);
    }
    for key in super::remembered_extra_keys(&dialog.kind) {
        if let Some(value) = saved.get("extra").and_then(|e| e.get(*key)).filter(|v| bounded(v, 0)) {
            dialog.extra.insert((*key).into(), value.clone());
        }
    }
    if let Some(through) = saved["lasso"]["through"].as_bool() {
        let mode = saved["lasso"]["mode"].as_u64().filter(|m| *m <= 2).unwrap_or(0) as usize;
        dialog.facet_lasso.restore_options(through, mode);
    }
    dialog.scan_copy.restore_memory(&saved["scan"]);
    dialog.scan_surface_fit.restore_memory(&saved["scan_fit"]);
    if saved.get("to_object").and_then(Value::as_bool) == Some(true) {
        super::preset_to_object(dialog, true);
    }
}
