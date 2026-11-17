//! Independent XT text export of placed manifold CAD solids.
//!
//! Record layout: UGS Parasolid XT Format Reference (October 2006), schema 13006.
//! Exact geometry carriers and shared topology; no tessellation is substituted.
use crate::{IoError, Result};
use solidcore_record::brep::{Curve, Loop, Shell, Surface};
use solidcraft_doc::ModelBody;
#[path = "parasolid_out/assembly.rs"]
pub(super) mod assembly;
#[path = "parasolid_out/geometry.rs"]
mod geometry;

#[derive(Clone)]
enum Field {
    I(usize),
    R(f64),
    C(char),
    L(bool),
    P(usize),
    V([f64; 3]),
    NullReal,
}
struct Writer {
    next: usize,
    out: String,
    nodes: Vec<Node>,
}
struct Node {
    kind: u16,
    id: usize,
    fields: Vec<Field>,
    range: std::ops::Range<usize>,
}
impl Writer {
    fn reserve(&mut self) -> usize {
        let id = self.next;
        self.next += 1;
        id
    }
    fn fields(&mut self, fields: Vec<Field>) -> Result<()> {
        for f in fields {
            match f {
                Field::I(n) | Field::P(n) => self.out.push_str(&format!("{n} ")),
                Field::R(v) => {
                    if !v.is_finite() {
                        return Err(unsupported("non-finite geometric value"));
                    }
                    self.out.push_str(&format!("{v} "));
                }
                Field::C(c) => self.out.push(c),
                Field::L(b) => self.out.push(if b { 'T' } else { 'F' }),
                Field::V(v) => {
                    for x in v {
                        if !x.is_finite() {
                            return Err(unsupported("non-finite vector"));
                        }
                        self.out.push_str(&format!("{x} "));
                    }
                }
                Field::NullReal => self.out.push('?'),
            }
        }
        Ok(())
    }
    fn node(&mut self, kind: u16, id: usize, fields: Vec<Field>) -> Result<()> {
        let start = self.out.len();
        self.out.push_str(&format!("{kind} {id} "));
        self.fields(fields.clone())?;
        self.nodes.push(Node { kind, id, fields, range: start..self.out.len() });
        Ok(())
    }
    fn array(&mut self, kind: u16, id: usize, fields: Vec<Field>) -> Result<()> {
        self.out.push_str(&format!("{kind} {} {id} ", fields.len()));
        self.fields(fields)
    }
}

fn unsupported(why: &str) -> IoError {
    IoError::Invalid(format!("Parasolid export cannot represent this body exactly: {why}. No faceted substitute was exported."))
}

struct Fin {
    loop_id: usize,
    next: usize,
    previous: usize,
    edge: usize,
    start: usize,
    end: usize,
    forward: bool,
    pcurve: solidcore_record::brep::Curve2,
}

pub(crate) fn export(bodies: &[&ModelBody]) -> Result<Vec<u8>> {
    if bodies.is_empty() {
        return Err(unsupported("no CAD bodies selected"));
    }
    let mut writer =
        Writer { nodes: Vec::new(), next: 2, out: String::from("T51 : TRANSMIT FILE created by modeller version 130000017 SCH_1300000_130060 ") };
    let ids: Vec<_> = bodies.iter().map(|_| writer.reserve()).collect();
    // A root pointer-list transmits all independent bodies without welding them.
    writer.out.push_str(&format!("74 {} 1 {} 0 ", ids.len(), ids.len()));
    for id in &ids {
        writer.out.push_str(&format!("{id} "));
    }
    for (model, id) in bodies.iter().zip(ids) {
        emit_body(&mut writer, model, id)?;
    }
    finish(writer)
}
fn finish(mut writer: Writer) -> Result<Vec<u8>> {
    let old = std::mem::take(&mut writer.out);
    let nodes = std::mem::take(&mut writer.nodes);
    let mut at = 0;
    for node in nodes {
        writer.out.push_str(&old[at..node.range.start]);
        writer.out.push_str(&format!("{} {} ", node.kind, node.id));
        writer.fields(node.fields)?;
        at = node.range.end;
    }
    writer.out.push_str(&old[at..]);
    writer.out.push_str("1 0 ");
    let mut bytes = Vec::with_capacity(writer.out.len() + writer.out.len() / 79 + 1);
    for line in writer.out.as_bytes().chunks(79) {
        bytes.extend_from_slice(line);
        bytes.push(b'\n');
        if line.last() == Some(&b' ') {
            bytes.push(b' ');
        }
    }
    Ok(bytes)
}
fn emit_body(w: &mut Writer, model: &ModelBody, body_id: usize) -> Result<()> {
    emit_body_instance(w, model, body_id, 0, 0)
}
fn emit_body_instance(w: &mut Writer, model: &ModelBody, body_id: usize, ref_instance: usize, attribute: usize) -> Result<()> {
    if model.body.is_mesh() {
        return Err(unsupported("a scan mesh is not a CAD solid"));
    }
    let record = model.body.record();
    record.validate().map_err(|e| IoError::Invalid(e.to_string()))?;
    // Every closed boundary has one material side and one void side. Preserve the
    // bounded void regions too: omitting them fills cavities on exchange.
    let material_regions: Vec<_> = record.regions.iter().enumerate().skip(1).filter(|(_, r)| !r.void).map(|(i, _)| i).collect();
    let [material_region] = material_regions.as_slice() else {
        return Err(unsupported("the body must have one material region"));
    };
    if record.faces.is_empty() || record.shells.is_empty() {
        return Err(unsupported("empty solid boundary"));
    }
    let mut uses = Vec::new();
    let mut face_shells = Vec::new();
    let mut shell_ranges = Vec::new();
    let mut void_regions = Vec::new();
    for (si, source_shell) in record.shells.iter().enumerate() {
        let Shell::Faces { faces, wires } = source_shell else {
            return Err(unsupported("non-face shell"));
        };
        if !wires.is_empty() || faces.is_empty() {
            return Err(unsupported("wire or empty shell"));
        }
        let members: Vec<_> = record.regions.iter().enumerate().filter(|(_, r)| r.shells.contains(&(si as u32))).map(|(i, _)| i).collect();
        if members.len() != 2 || !members.contains(material_region) {
            return Err(unsupported("open or non-material shell boundary"));
        }
        let other = members.iter().copied().find(|i| i != material_region).ok_or_else(|| unsupported("missing void region"))?;
        if other != 0 && !record.regions[other].void {
            return Err(unsupported("non-void cavity region"));
        }
        void_regions.push(other);
        let first = uses.len();
        uses.extend(faces.iter().copied());
        face_shells.extend(std::iter::repeat_n(si, faces.len()));
        shell_ranges.push(first..uses.len());
    }
    if uses.len() != record.faces.len() {
        return Err(unsupported("incomplete or repeated face boundary"));
    }
    if record.faces.len() + record.edges.len() + record.vertices.len() > 100_000 {
        return Err(unsupported("the export exceeds the 100,000 topology-item limit"));
    }
    let mut fins = Vec::new();
    let mut loops = Vec::new();
    let mut faces = Vec::new();
    let mut edge_fins = vec![Vec::new(); record.edges.len()];
    let mut vertex_fins = vec![None; record.vertices.len()];
    for use_face in &uses {
        let face_index = use_face.face as usize;
        let face = record.faces.get(face_index).ok_or_else(|| unsupported("invalid face reference"))?;

        let first_loop = loops.len();
        for boundary in &face.loops {
            let Loop::Edges { edges } = boundary else {
                return Err(unsupported("a collapsed boundary"));
            };
            if edges.is_empty() {
                return Err(unsupported("empty face boundary"));
            }
            let loop_id = loops.len();
            let first = fins.len();
            let mut oriented: Vec<_> = edges.iter().map(|e| (e.edge as usize, e.forward, e.pcurve.clone())).collect();
            if !use_face.forward {
                oriented.reverse();
                for (_, forward, _) in &mut oriented {
                    *forward = !*forward;
                }
            }
            let n_oriented = oriented.len();
            for (position, (index, forward, pcurve)) in oriented.into_iter().enumerate() {
                let edge = record.edges.get(index).ok_or_else(|| unsupported("invalid edge reference"))?;

                let [a, b] = edge.vertices;
                let (start, end) = if forward { (a as usize, b as usize) } else { (b as usize, a as usize) };
                edge_fins.get_mut(index).ok_or_else(|| unsupported("invalid edge reference"))?.push(fins.len());
                vertex_fins.get_mut(end).ok_or_else(|| unsupported("invalid vertex reference"))?.get_or_insert(fins.len());
                fins.push(Fin {
                    loop_id,
                    next: first + (position + 1) % n_oriented,
                    previous: first + (position + n_oriented - 1) % n_oriented,
                    edge: index,
                    start,
                    end,
                    forward,
                    pcurve,
                });
            }
            loops.push((faces.len(), first));
        }
        if loops.len() == first_loop {
            return Err(unsupported("unbounded face"));
        }
        faces.push((face_index, use_face.forward, first_loop, loops.len()));
    }
    for (index, fin) in fins.iter().enumerate() {
        let pair = edge_fins.get(fin.edge).ok_or_else(|| unsupported("missing edge uses"))?;
        let [a, b] = pair.as_slice() else {
            return Err(unsupported("open or non-manifold edges"));
        };
        let other = fins.get(if *a == index { *b } else { *a }).ok_or_else(|| unsupported("invalid edge use"))?;
        let next = fins.get(fin.next).ok_or_else(|| unsupported("invalid loop"))?;
        if fin.start != other.end || fin.end != other.start || fin.end != next.start {
            return Err(unsupported("inconsistent boundary orientation"));
        }
    }
    use Field::{C, I, NullReal as Q, P, R, V};
    let node_start = w.nodes.len();
    let region_ids: Vec<_> = record.regions.iter().map(|_| w.reserve()).collect();
    let material_shell_ids: Vec<_> = record.shells.iter().map(|_| w.reserve()).collect();
    let void_shell_ids: Vec<_> = record.shells.iter().map(|_| w.reserve()).collect();
    let outside = region_ids[0];
    let shell = material_shell_ids[0];
    let face_ids: Vec<_> = faces.iter().map(|_| w.reserve()).collect();
    let loop_ids: Vec<_> = loops.iter().map(|_| w.reserve()).collect();
    let surface_ids: Vec<_> = faces.iter().map(|_| w.reserve()).collect();
    let fin_ids: Vec<_> = fins.iter().map(|_| w.reserve()).collect();
    let edge_ids: Vec<_> = record.edges.iter().map(|_| w.reserve()).collect();
    let curve_ids: Vec<_> = record.edges.iter().map(|_| w.reserve()).collect();
    let vertex_ids: Vec<_> = record.vertices.iter().map(|_| w.reserve()).collect();
    let point_ids: Vec<_> = record.vertices.iter().map(|_| w.reserve()).collect();
    for (ri, source_region) in record.regions.iter().enumerate() {
        let sides: Vec<_> = source_region
            .shells
            .iter()
            .map(|si| {
                let si = *si as usize;
                if ri == *material_region { material_shell_ids[si] } else { void_shell_ids[si] }
            })
            .collect();
        let id = region_ids[ri];
        w.node(
            19,
            id,
            vec![
                I(id + 1),
                P(0),
                P(body_id),
                P(region_ids.get(ri + 1).copied().unwrap_or(0)),
                P(ri.checked_sub(1).map_or(0, |i| region_ids[i])),
                P(sides.first().copied().unwrap_or(0)),
                C(if ri == *material_region { 'S' } else { 'V' }),
            ],
        )?;
        for (position, si) in source_region.shells.iter().enumerate() {
            let si = *si as usize;
            let first_face = face_ids[shell_ranges[si].start];
            let material = ri == *material_region;
            if !material && void_regions[si] != ri {
                return Err(unsupported("inconsistent void-side ownership"));
            }
            let id = sides[position];
            w.node(
                13,
                id,
                vec![
                    I(id + 1),
                    P(0),
                    P(if material { body_id } else { 0 }),
                    P(sides.get(position + 1).copied().unwrap_or(0)),
                    P(if material { first_face } else { 0 }),
                    P(0),
                    P(0),
                    P(region_ids[ri]),
                    P(if material { 0 } else { first_face }),
                ],
            )?;
        }
    }
    for (i, (source, forward, first, last)) in faces.iter().enumerate() {
        let id = face_ids[i];
        let face = &record.faces[*source];
        let si = face_shells[i];
        let range = &shell_ranges[si];
        let next_face = if i + 1 < range.end { face_ids[i + 1] } else { 0 };
        let previous_face = if i > range.start { face_ids[i - 1] } else { 0 };
        w.node(
            14,
            id,
            vec![
                I(id + 1),
                P(0),
                Q,
                P(next_face),
                P(previous_face),
                P(loop_ids[*first]),
                P(material_shell_ids[si]),
                P(surface_ids[i]),
                C(if *forward { '+' } else { '-' }),
                P(0),
                P(0),
                P(next_face),
                P(previous_face),
                P(void_shell_ids[si]),
            ],
        )?;
        geometry::surface(w, surface_ids[i], id, face.nurbs.as_ref().unwrap_or(&face.surface))?;
        for j in *first..*last {
            let (_, first_fin) = loops[j];
            let id = loop_ids[j];
            w.node(15, id, vec![I(id + 1), P(0), P(fin_ids[first_fin]), P(face_ids[i]), P(if j + 1 < *last { loop_ids[j + 1] } else { 0 })])?;
        }
    }
    for (i, fin) in fins.iter().enumerate() {
        let mate = *edge_fins[fin.edge].iter().find(|other| **other != i).ok_or_else(|| unsupported("missing mate"))?;
        let face_index = loops[fin.loop_id].0;
        let source = &record.faces[faces[face_index].0];
        let surface = source.nurbs.as_ref().unwrap_or(&source.surface);
        let (scale, offset) = uv_mapping(surface)?;
        let sp = w.reserve();
        let bounded = w.reserve();
        let uv = w.reserve();
        geometry::pcurve(w, uv, 0, &fin.pcurve, scale, offset)?;
        w.node(
            137,
            sp,
            vec![
                I(sp + 1),
                P(0),
                P(body_id),
                P(0),
                P(0),
                P(0),
                C('+'),
                P(surface_ids[face_index]),
                P(uv),
                P(curve_ids[fin.edge]),
                R(record.edges[fin.edge].tolerance * 0.001),
            ],
        )?;
        let edge = &record.edges[fin.edge];
        let ends = surface_trim_ends(surface, &fin.pcurve, edge.interval)?;
        w.node(
            133,
            bounded,
            vec![
                I(bounded + 1),
                P(0),
                P(fin_ids[i]),
                P(0),
                P(0),
                P(0),
                C('+'),
                P(sp),
                V(ends[0].map(|x| x * 0.001)),
                V(ends[1].map(|x| x * 0.001)),
                R(edge.interval[0]),
                R(edge.interval[1]),
            ],
        )?;
        w.node(
            17,
            fin_ids[i],
            vec![
                P(0),
                P(loop_ids[fin.loop_id]),
                P(fin_ids[fin.next]),
                P(fin_ids[fin.previous]),
                P(vertex_ids[fin.end]),
                P(fin_ids[mate]),
                P(edge_ids[fin.edge]),
                P(bounded),
                P(fins.iter().enumerate().skip(i + 1).find(|(_, f)| f.end == fin.end).map_or(0, |(j, _)| fin_ids[j])),
                C(if fin.forward { '+' } else { '-' }),
            ],
        )?;
    }
    for (i, edge) in record.edges.iter().enumerate() {
        let first = *edge_fins[i].iter().find(|n| fins[**n].forward).ok_or_else(|| unsupported("missing forward edge use"))?;
        let id = edge_ids[i];
        w.node(
            16,
            id,
            vec![
                I(id + 1),
                P(0),
                R(edge.tolerance * 0.001),
                P(fin_ids[first]),
                P(i.checked_sub(1).map_or(0, |j| edge_ids[j])),
                P(edge_ids.get(i + 1).copied().unwrap_or(0)),
                P(0),
                P(0),
                P(0),
                P(body_id),
            ],
        )?;
        let carrier = w.reserve();
        let curve = edge.nurbs.as_ref().unwrap_or(&edge.curve);
        let ends = curve_ends(curve, edge.interval)?;
        let mut interval = edge.interval;
        if let Curve::Line { start, end } = curve {
            geometry::line_domain(w, carrier, body_id, *start, *end, interval)?;
        } else {
            geometry::curve(w, carrier, body_id, curve)?;
        }
        if let Curve::Ellipse { radii, .. } = curve {
            if radii[1] > radii[0] {
                interval = interval.map(|t| t - std::f64::consts::FRAC_PI_2);
            }
        }
        w.node(
            133,
            curve_ids[i],
            vec![
                I(curve_ids[i] + 1),
                P(0),
                P(id),
                P(0),
                P(0),
                P(0),
                C('+'),
                P(carrier),
                V(ends[0].map(|x| x * 0.001)),
                V(ends[1].map(|x| x * 0.001)),
                R(interval[0]),
                R(interval[1]),
            ],
        )?;
    }
    for (i, vertex) in record.vertices.iter().enumerate() {
        let fin = vertex_fins[i].ok_or_else(|| unsupported("unused vertex"))?;
        let id = vertex_ids[i];
        w.node(
            18,
            id,
            vec![
                I(id + 1),
                P(0),
                P(fin_ids[fin]),
                P(i.checked_sub(1).map_or(0, |j| vertex_ids[j])),
                P(vertex_ids.get(i + 1).copied().unwrap_or(0)),
                P(point_ids[i]),
                R(vertex.tolerance * 0.001),
                P(body_id),
            ],
        )?;
        w.node(29, point_ids[i], vec![I(point_ids[i] + 1), P(0), P(id), P(0), P(0), V(vertex.point.map(|x| x * 0.001))])?;
    }
    let heads = link_geometry(w, node_start, body_id)?;
    w.node(
        12,
        body_id,
        vec![
            I(w.next + 1),
            P(attribute),
            P(0),
            P(0),
            P(0),
            P(0),
            P(0),
            R(1000.),
            R(record.tolerance * 0.001),
            P(ref_instance),
            P(0),
            P(0),
            I(1),
            P(0),
            I(1),
            I(1),
            P(shell),
            P(heads[0]),
            P(heads[1]),
            P(heads[2]),
            P(outside),
            P(edge_ids[0]),
            P(vertex_ids[0]),
        ],
    )?;
    Ok(())
}

fn uv_mapping(surface: &Surface) -> Result<([f64; 2], [f64; 2])> {
    let norm = |v: [f64; 3]| v.iter().map(|x| x * x).sum::<f64>().sqrt();
    let plane = |u: [f64; 3], v: [f64; 3]| -> Result<([f64; 2], [f64; 2])> {
        let nu = norm(u);
        let nv = norm(v);
        let dot = u.iter().zip(v).map(|(a, b)| a * b).sum::<f64>();
        if nu <= 0. || nv <= 0. || dot.abs() > nu * nv * 1e-10 {
            return Err(unsupported("nonorthogonal plane parameters"));
        }
        Ok(([nu * 0.001, nv * 0.001], [0.; 2]))
    };
    match surface {
        Surface::Plane { u_axis, v_axis, .. } => plane(*u_axis, *v_axis),
        Surface::PlaneThrough { origin, u_point, v_point } => {
            plane(std::array::from_fn(|i| u_point[i] - origin[i]), std::array::from_fn(|i| v_point[i] - origin[i]))
        }
        Surface::Cylinder { .. } => Ok(([1., 0.001], [0.; 2])),
        Surface::Cone { half_angle, .. } => Ok((if *half_angle > 0. { [-1., 0.001] } else { [1., -0.001] }, [0.; 2])),
        Surface::Torus { tube_start, .. } => Ok(([1.; 2], [0., *tube_start])),
        Surface::Sphere { .. } | Surface::Spline { .. } | Surface::RationalSpline { .. } => Ok(([1.; 2], [0.; 2])),
        _ => Err(unsupported("unsupported surface parameters")),
    }
}

// Restore the bidirectional geometry ownership/chains required by XT, including
// indirect support surfaces and trimmed basis curves (reference section 7.5).
fn link_geometry(w: &mut Writer, start: usize, body: usize) -> Result<[usize; 3]> {
    use Field::P;
    let pointer = |f: &Field| if let P(id) = f { *id } else { 0 };
    let mut chains: [Vec<usize>; 3] = std::array::from_fn(|_| Vec::new());
    let mut dependencies = std::collections::BTreeMap::<usize, Vec<usize>>::new();
    let edge_ids: std::collections::BTreeSet<_> = w.nodes[start..].iter().filter(|n| n.kind == 16).map(|n| n.id).collect();
    for i in start..w.nodes.len() {
        let n = &mut w.nodes[i];
        let class = match n.kind {
            50..=54 | 124 => Some(0),
            30..=32 | 133 | 134 | 137 => Some(1),
            29 => Some(2),
            _ => None,
        };
        let Some(class) = class else { continue };
        let owner = pointer(&n.fields[2]);
        if n.kind == 134 && owner == 0 {
            continue;
        }
        if edge_ids.contains(&owner) {
            n.fields[2] = P(body);
        }
        chains[class].push(i);
        if matches!(n.kind, 133 | 137) {
            dependencies.entry(pointer(&n.fields[7])).or_default().push(n.id);
        }
    }
    let mut heads = [0; 3];
    for (class, chain) in chains.iter().enumerate() {
        heads[class] = chain.first().map_or(0, |i| w.nodes[*i].id);
        for (position, &i) in chain.iter().enumerate() {
            let prev = position.checked_sub(1).map_or(0, |j| w.nodes[chain[j]].id);
            let next = chain.get(position + 1).map_or(0, |j| w.nodes[*j].id);
            w.nodes[i].fields[3] = P(next);
            w.nodes[i].fields[4] = P(prev);
        }
    }
    for (shared, owners) in dependencies {
        let refs: Vec<_> = owners.iter().map(|_| w.reserve()).collect();
        let node = w.nodes.iter_mut().skip(start).find(|n| n.id == shared).ok_or_else(|| unsupported("missing dependent geometry"))?;
        node.fields[5] = P(refs[0]);
        for (i, owner) in owners.iter().enumerate() {
            w.node(141, refs[i], vec![P(*owner), P(refs[(i + 1) % refs.len()]), P(refs[(i + refs.len() - 1) % refs.len()]), P(shared)])?;
        }
    }
    Ok(heads)
}

fn curve_ends(curve: &Curve, interval: [f64; 2]) -> Result<[[f64; 3]; 2]> {
    let eval = solidcraft_geom::brep::eval::CurveEval::new(curve).map_err(|e| unsupported(&format!("curve evaluation: {e}")))?;
    let point = |t| eval.point(t).map(|p| [p.x, p.y, p.z]).ok_or_else(|| unsupported("curve endpoint cannot be evaluated"));
    Ok([point(interval[0])?, point(interval[1])?])
}
fn surface_trim_ends(surface: &Surface, pcurve: &solidcore_record::brep::Curve2, interval: [f64; 2]) -> Result<[[f64; 3]; 2]> {
    use solidcore_record::brep::Curve2;
    let uv_curve = match pcurve {
        Curve2::Spline { knots, points } => Curve::Spline { knots: knots.clone(), points: points.iter().map(|p| [p[0], p[1], 0.]).collect() },
        Curve2::RationalSpline { knots, homogeneous } => {
            Curve::RationalSpline { knots: knots.clone(), homogeneous: homogeneous.iter().map(|p| [p[0], p[1], 0., p[2]]).collect() }
        }
    };
    let uv = curve_ends(&uv_curve, interval)?;
    let eval = solidcraft_geom::brep::eval::SurfaceEval::new(surface).map_err(|e| unsupported(&format!("surface evaluation: {e}")))?;
    let point =
        |p: [f64; 3]| eval.point([p[0], p[1]]).map(|p| [p.x, p.y, p.z]).ok_or_else(|| unsupported("surface trim endpoint cannot be evaluated"));
    Ok([point(uv[0])?, point(uv[1])?])
}
