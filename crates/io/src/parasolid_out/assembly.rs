//! XT assembly topology and standard name attributes. Bodies are already placed
//! in world coordinates, so instance transforms are identity. This deliberately
//! duplicates placed definitions rather than losing instance placement or grouping.
use super::{Field as F, Writer, emit_body_instance, finish, unsupported};
use crate::Result;
use solidcraft_doc::{Document, ModelState, WorldNames};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug)]
struct Group {
    path: Vec<u64>,
    name: String,
    bodies: Vec<usize>,
}
#[derive(Debug)]
struct Part {
    name: String,
    body: Option<usize>,
    children: Vec<usize>,
}

fn groups(doc: &Document, local: &ModelState, world: &ModelState, name: &str) -> Result<Vec<Group>> {
    let owners = doc.model_body_components(local);
    let names = WorldNames::new(doc, local);
    let by_name: BTreeMap<_, _> = world.bodies.iter().enumerate().map(|(i, b)| (b.name.as_str(), i)).collect();
    if by_name.len() != world.bodies.len() {
        return Err(unsupported("world body names are not unique"));
    }
    let mut grouped: BTreeMap<Vec<u64>, Vec<usize>> = BTreeMap::new();
    grouped.insert(Vec::new(), Vec::new());
    let mut used = BTreeSet::new();
    for (component, path, _) in doc.placements() {
        for (body, owner) in local.bodies.iter().zip(&owners) {
            if *owner != component {
                continue;
            }
            let world_name = names.name(&body.name, component, &path);
            if let Some(&index) = by_name.get(world_name.as_str()) {
                if !used.insert(index) {
                    return Err(unsupported("one world body belongs to multiple occurrence placements"));
                }
                grouped.entry(path.clone()).or_default().push(index);
                for length in 0..path.len() {
                    grouped.entry(path[..length].to_vec()).or_default();
                }
            }
        }
    }
    if used.len() != world.bodies.len() {
        return Err(unsupported("a placed body has no resolvable component occurrence; assembly grouping would be lost"));
    }
    let mut result = Vec::with_capacity(grouped.len());
    for (path, bodies) in grouped {
        let label = if let Some(last) = path.last() {
            doc.occurrences
                .iter()
                .find(|o| o.id == *last)
                .map(|o| o.name.clone())
                .ok_or_else(|| unsupported("assembly occurrence name cannot be resolved"))?
        } else {
            name.to_owned()
        };
        result.push(Group { path, name: label, bodies });
    }
    Ok(result)
}

/// Single-body leaves are BODY parts. Components with children or several bodies
/// are ASSEMBLY parts; each owned body remains a distinct child, never welded.
fn plan(groups: &[Group], body_names: &[String]) -> Result<Vec<Part>> {
    if groups.first().is_none_or(|g| !g.path.is_empty()) {
        return Err(unsupported("assembly has no root group"));
    }
    let lookup: BTreeMap<_, _> = groups.iter().enumerate().map(|(i, g)| (g.path.clone(), i)).collect();
    let mut children = vec![Vec::new(); groups.len()];
    for (i, g) in groups.iter().enumerate().skip(1) {
        let parent = lookup.get(&g.path[..g.path.len() - 1]).ok_or_else(|| unsupported("assembly is missing a parent occurrence"))?;
        children[*parent].push(i);
    }
    let mut parts: Vec<_> = groups
        .iter()
        .enumerate()
        .map(|(i, g)| Part {
            name: g.name.clone(),
            body: if i > 0 && children[i].is_empty() && g.bodies.len() == 1 { Some(g.bodies[0]) } else { None },
            children: children[i].clone(),
        })
        .collect();
    for (i, g) in groups.iter().enumerate() {
        if parts[i].body.is_some() {
            continue;
        }
        for &body in &g.bodies {
            let name = body_names.get(body).ok_or_else(|| unsupported("assembly body index is invalid"))?.clone();
            let index = parts.len();
            parts.push(Part { name, body: Some(body), children: Vec::new() });
            parts[i].children.push(index);
        }
    }
    Ok(parts)
}

/// The public entry point accepts local geometry only for ownership lookup. The
/// supplied world state is authoritative for final geometry, including pending moves.
pub(crate) fn export(doc: &Document, local: &ModelState, world: &ModelState, name: &str) -> Result<Vec<u8>> {
    if world.bodies.is_empty() {
        return Err(unsupported("no CAD bodies selected"));
    }
    let groups = groups(doc, local, world, name)?;
    let parts = plan(&groups, &world.bodies.iter().map(|b| b.name.clone()).collect::<Vec<_>>())?;
    let mut w =
        Writer { nodes: Vec::new(), next: 2, out: String::from("T51 : TRANSMIT FILE created by modeller version 130000017 SCH_1300000_130060 ") };
    let ids: Vec<_> = parts.iter().map(|_| w.reserve()).collect();
    let attrs: Vec<_> = parts.iter().map(|_| w.reserve()).collect();
    let mut incoming = vec![0; parts.len()];
    let mut instances = Vec::new();
    for (parent, part) in parts.iter().enumerate() {
        let links: Vec<_> = part.children.iter().map(|_| w.reserve()).collect();
        for (j, &child) in part.children.iter().enumerate() {
            if incoming[child] != 0 {
                return Err(unsupported("world-space part unexpectedly has multiple parent instances"));
            }
            incoming[child] = links[j];
            instances.push((links[j], parent, child, j.checked_sub(1).map_or(0, |p| links[p]), links.get(j + 1).copied().unwrap_or(0), w.reserve()));
        }
    }
    // Transmit exactly the root assembly. Child parts are reachable through instances.
    w.out.push_str(&format!("74 1 1 1 0 {} ", ids[0]));
    let definition = name_definition(&mut w)?;
    for (i, part) in parts.iter().enumerate() {
        name_attribute(&mut w, attrs[i], ids[i], definition, &part.name)?;
        if let Some(body) = part.body {
            emit_body_instance(&mut w, &world.bodies[body], ids[i], incoming[i], attrs[i])?;
        }
    }
    for &(id, parent, child, previous, next, attribute) in &instances {
        name_attribute(&mut w, attribute, id, definition, &parts[child].name)?;
        w.node(
            11,
            id,
            vec![F::I(id + 1), F::P(attribute), F::I(1), F::P(ids[child]), F::P(0), F::P(ids[parent]), F::P(next), F::P(previous), F::P(0), F::P(0)],
        )?;
    }
    let highest = w.next + 1;
    for (i, _) in parts.iter().enumerate().filter(|(_, p)| p.body.is_none()) {
        let first = instances.iter().find(|(_, parent, _, _, _, _)| *parent == i).map_or(0, |v| v.0);
        w.node(
            10,
            ids[i],
            vec![
                F::I(highest),
                F::P(attrs[i]),
                F::P(0),
                F::P(0),
                F::P(0),
                F::P(0),
                F::P(0),
                F::P(0),
                F::R(1000.),
                F::R(1e-8),
                F::P(incoming[i]),
                F::P(0),
                F::P(0),
                F::I(1),
                F::P(0),
                F::I(1),
                F::P(first),
            ],
        )?;
    }
    finish(w)
}

/// The 13006 definition contains one variable field, not one entry for every
/// fixed field; variable-array record headers cannot use Writer::array directly.
fn name_definition(w: &mut Writer) -> Result<usize> {
    let id = w.reserve();
    let key = w.reserve();
    chars(w, 79, key, "SDL/TYSA_NAME");
    w.out.push_str(&format!("80 1 {id} "));
    let mut fields = vec![F::P(0), F::P(key), F::I(8017)];
    fields.extend([0, 0, 0, 0, 0, 5, 0, 0].into_iter().map(F::I));
    fields.push(F::P(0));
    fields.extend((0..14).map(|i| F::L(i < 12)));
    fields.push(F::I(3));
    w.fields(fields)?;
    Ok(id)
}
fn chars(w: &mut Writer, kind: u16, id: usize, name: &str) {
    // Text character arrays are byte-counted. Keep UTF-8 intact, remove only
    // control characters which would otherwise interfere with physical wrapping.
    let clean: String = name.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let escaped = clean.replace('\\', "\\\\");
    w.out.push_str(&format!("{kind} {} {id} {escaped}", clean.len()));
}
fn name_attribute(w: &mut Writer, id: usize, owner: usize, definition: usize, name: &str) -> Result<()> {
    let value = w.reserve();
    chars(w, 84, value, name);
    w.out.push_str(&format!("81 1 {id} "));
    w.fields(vec![F::I(id + 1), F::P(definition), F::P(owner), F::P(0), F::P(0), F::P(0), F::P(0), F::P(value)])
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_and_multibody_components_keep_separate_part_nodes() -> Result<()> {
        let groups = vec![
            Group { path: vec![], name: "Assembly".into(), bodies: vec![0] },
            Group { path: vec![10], name: "Bracket".into(), bodies: vec![1] },
            Group { path: vec![20], name: "Module".into(), bodies: vec![2, 3] },
            Group { path: vec![20, 30], name: "Fastener".into(), bodies: vec![4] },
        ];
        let p = plan(&groups, &(0..5).map(|n| format!("Body{n}")).collect::<Vec<_>>())?;
        assert_eq!(p.iter().filter(|p| p.body.is_some()).count(), 5);
        assert_eq!(p.iter().filter(|p| p.body.is_none()).count(), 2);
        assert_eq!(p[1].body, Some(1));
        assert_eq!(p[1].name, "Bracket");
        assert!(p[2].body.is_none());
        assert_eq!(p[2].children.len(), 3);
        assert_eq!(p[3].body, Some(4));
        assert_eq!(p[3].name, "Fastener");
        let all: BTreeSet<_> = p.iter().filter_map(|p| p.body).collect();
        assert_eq!(all.len(), 5);
        Ok(())
    }
    #[test]
    fn repeated_occurrences_are_distinct_named_placed_parts() -> Result<()> {
        let groups = vec![
            Group { path: vec![], name: "root".into(), bodies: vec![] },
            Group { path: vec![1], name: "Left".into(), bodies: vec![0] },
            Group { path: vec![2], name: "Right".into(), bodies: vec![1] },
        ];
        let p = plan(&groups, &["Body1".into(), "Body1 (Right)".into()])?;
        assert_eq!(p[0].children, [1, 2]);
        assert_eq!(p[1].body, Some(0));
        assert_eq!(p[2].body, Some(1));
        assert_ne!(p[1].name, p[2].name);
        Ok(())
    }
    #[test]
    fn missing_parent_is_rejected() {
        assert!(
            plan(
                &[Group { path: vec![], name: "root".into(), bodies: vec![] }, Group { path: vec![1, 2], name: "bad".into(), bodies: vec![0] }],
                &["body".into()]
            )
            .is_err()
        );
    }
}
