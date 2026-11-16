//! Cached local mobility analysis for spatial sketch presentation. A redraw never reruns
//! the numerical analysis unless editable geometry or constraints changed.
use crate::SolveApp;
use solidcraft_engine::sketch::{Sketch, wire_constraints::SpatialFreedom};
use std::{
    cell::RefCell,
    collections::{VecDeque, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    rc::Rc,
};

thread_local! {
    static CACHE: RefCell<VecDeque<(u64, Rc<SpatialFreedom>)>> = const { RefCell::new(VecDeque::new()) };
}

pub(crate) fn analysis(sketch: &Sketch) -> Rc<SpatialFreedom> {
    let mut h = DefaultHasher::new();
    sketch.wires.len().hash(&mut h);
    for wire in &sketch.wires {
        wire.id.hash(&mut h);
        wire.link.hash(&mut h);
        (wire.space == solidcraft_engine::sketch::WireSpace::Component).hash(&mut h);
        wire.arc.is_some().hash(&mut h);
        wire.fit.len().hash(&mut h);
        for p in &wire.fit {
            p.x.to_bits().hash(&mut h);
            p.y.to_bits().hash(&mut h);
            p.z.to_bits().hash(&mut h);
        }
    }
    if let Ok(bytes) = serde_json::to_vec(&sketch.wire_constraints) {
        bytes.hash(&mut h);
    }
    let key = h.finish();
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some((_, report)) = cache.iter().find(|(k, _)| *k == key) {
            return Rc::clone(report);
        }
        let report = Rc::new(sketch.wire_freedom());
        if cache.len() >= 16 {
            cache.pop_front();
        }
        cache.push_back((key, Rc::clone(&report)));
        report
    })
}

pub(crate) fn active(app: &SolveApp) -> Option<Rc<SpatialFreedom>> {
    let state = app.session.model.state();
    Some(analysis(&state.sketch(app.session.active_sketch?)?.sketch))
}

pub(crate) fn remaining(sketch: &Sketch, planar: usize) -> Option<usize> {
    analysis(sketch).dof.map(|n| n.saturating_add(planar))
}

pub(crate) fn status(sketch: &Sketch, planar: usize) -> String {
    match remaining(sketch, planar) {
        Some(0) => solidcraft_ui_text::tr("ui.fully_constrained").to_string(),
        Some(n) => solidcraft_ui_text::format("ui.spatial_freedom_remaining", &[("count", &n.to_string())]),
        None => solidcraft_ui_text::tr("ui.spatial_dof_unknown").to_string(),
    }
}
