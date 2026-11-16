// SPDX-License-Identifier: Apache-2.0

use super::{candidate_has_valid_family_framing, FixedRecordSelection, Graph, Node, NodeCandidate, NodeKind};
use cadmpeg_core::CodecError;
use cadmpeg_parasolid::records::primary::PrimaryRecord;
use cadmpeg_parasolid::records::{EdgeFrame, PrimaryRecords};
use cadmpeg_parasolid::xmt::NonNullXmt;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

/// Primary framing of one stream, which outranks the fixed-record scan.
pub(super) struct PrimaryFrames {
    pub(super) edges: Vec<EdgeFrame>,
    records: Vec<Range<usize>>,
    /// Whether a terminator ends the node sequence, so every node of the
    /// stream is in `records`.
    terminated: bool,
}

impl PrimaryFrames {
    /// The node ranges of a terminated sequence.
    pub(super) fn terminated_records(&self) -> Option<&[Range<usize>]> {
        self.terminated.then_some(self.records.as_slice())
    }

    /// Whether primary framing assigns a candidate's bytes to another node.
    pub(super) fn exclude(&self, candidate: &NodeCandidate) -> bool {
        let index = self.records.partition_point(|record| record.end <= candidate.pos);
        let inside = self.records.get(index).is_some_and(|record| record.start < candidate.pos);
        inside || self.overlaps_edge(candidate)
    }

    fn overlaps_edge(&self, candidate: &NodeCandidate) -> bool {
        let index = self.edges.partition_point(|frame| frame.range.end <= candidate.pos);
        self.edges.get(index).is_some_and(|frame| frame.range.start < candidate.end)
    }
}

impl Graph {
    /// Use primary record ownership when the header is supported.
    pub(crate) fn parse_source(stream: &[u8]) -> Result<Self, CodecError> {
        let (frames, stop, limit_term_use, selection, named_ownership) = {
            let Some(primary) = PrimaryRecords::read(stream)? else {
                return Ok(Self::parse(stream));
            };
            let limit_term_use = crate::intersection::limit_term_use(&primary);
            let frames = PrimaryFrames {
                edges: primary.edge_frames(),
                records: primary.records().iter().map(PrimaryRecord::range).collect(),
                terminated: primary.stop_offset().is_none(),
            };
            let selection = frames.terminated.then(|| node_selection(stream, &primary));
            let named_ownership = if frames.terminated { named_ownership_nodes(stream, &primary) } else { Vec::new() };
            (frames, primary.stop_offset(), limit_term_use, selection, named_ownership)
        };
        let mut graph = match selection {
            Some((selection, supported)) => {
                let mut graph = selection.materialize(stream);
                for node in named_ownership {
                    let key = (node.kind, node.xmt());
                    graph.by_pos.insert(node.pos(), key);
                    graph.nodes.insert(key, node);
                }
                graph.index_kinds();
                graph.primary_node_refusals =
                    supported.into_iter().filter(|&(kind, start)| graph.by_pos.get(&start).is_none_or(|(selected, _)| *selected != kind)).collect();
                let refusals = frames
                    .edges
                    .iter()
                    .map(|frame| frame.range.start)
                    .filter(|start| graph.by_pos.get(start).is_none_or(|(kind, _)| *kind != NodeKind::Edge))
                    .collect();
                graph.primary_edge_refusals = refusals;
                graph
            }
            None => {
                let mut graph = Self::parse_fixed(stream, Some(&frames));
                graph.primary_edge_refusals = frames
                    .edges
                    .iter()
                    .filter(|frame| Self::primary_edge_candidate(stream, frame).is_none())
                    .map(|frame| frame.range.start)
                    .collect();
                graph
            }
        };
        graph.primary_unread_from = stop.filter(|offset| *offset < stream.len());
        graph.primary_records = frames.terminated_records().map(<[_]>::to_vec);
        graph.primary_limit_term_use = limit_term_use;
        Ok(graph)
    }

    /// Where the stream's auxiliary records start: the node starts of a
    /// terminated primary sequence, else every tag offset.
    pub(crate) fn record_starts(&self) -> crate::intersection::RecordStarts<'_> {
        self.primary_records.as_deref().map_or(crate::intersection::RecordStarts::Scan, |records| crate::intersection::RecordStarts::Nodes {
            records,
            limit_term_use: self.primary_limit_term_use,
        })
    }

    pub(crate) fn primary_edge_refusals(&self) -> &[usize] {
        &self.primary_edge_refusals
    }

    pub(crate) fn primary_node_refusals(&self) -> &[(NodeKind, usize)] {
        &self.primary_node_refusals
    }

    pub(crate) fn primary_unread_from(&self) -> Option<usize> {
        self.primary_unread_from
    }

    pub(super) fn primary_edge_candidate(stream: &[u8], frame: &EdgeFrame) -> Option<NodeCandidate> {
        let shift = frame.fixed_fields_at?.checked_sub(frame.range.start.checked_add(4)?)?;
        candidate_has_valid_family_framing(stream, frame.range.start, NodeKind::Edge, shift, frame.range.end, true)?;
        Some(NodeCandidate { kind: NodeKind::Edge, xmt: frame.xmt, pos: frame.range.start, shift, end: frame.range.end })
    }
}

/// The fixed records of a terminated node sequence: one candidate per
/// supported node with the base field layout, with an identity that two nodes
/// share left out before the family checks. The kind and start of every
/// supported non-EDGE node follow the selection.
fn node_selection(stream: &[u8], primary: &PrimaryRecords<'_>) -> (FixedRecordSelection, Vec<(NodeKind, usize)>) {
    let mut typed = Vec::new();
    let mut ownership = Vec::new();
    let mut supported = Vec::new();
    for node in primary.nodes() {
        let Some(kind) = u8::try_from(node.kind()).ok().and_then(|code| NodeKind::try_from(code).ok()) else {
            continue;
        };
        let range = node.range();
        if kind != NodeKind::Edge {
            supported.push((kind, range.start));
        }
        if !node.has_base_layout() {
            continue;
        }
        let Some(shift) = node.fields_start().zip(range.start.checked_add(4)).and_then(|(first, fields)| first.checked_sub(fields)) else {
            continue;
        };
        let Ok(xmt) = NonNullXmt::try_from(node.index()) else {
            continue;
        };
        let candidate = NodeCandidate { kind, xmt, pos: range.start, shift, end: range.end };
        if matches!(kind, NodeKind::Body | NodeKind::Region) {
            ownership.push(candidate);
        } else {
            typed.push(candidate);
        }
    }
    let valid = |candidates: Vec<NodeCandidate>| {
        Graph::select_unique_candidates(candidates)
            .into_iter()
            .filter(|candidate| {
                candidate_has_valid_family_framing(stream, candidate.pos, candidate.kind, candidate.shift, candidate.end, true).is_some()
            })
            .collect()
    };
    let selection = FixedRecordSelection { typed: valid(typed), ownership: valid(ownership) };
    (selection, supported)
}

/// Admit extended BODY/REGION layouts through their schema-named ownership
/// fields. A complete, unique body/region/shell graph is required; no field
/// offset or unsupported ownership edge is guessed.
fn named_ownership_nodes(stream: &[u8], primary: &PrimaryRecords<'_>) -> Vec<Node> {
    let mut nodes = BTreeMap::new();
    let mut duplicates = BTreeSet::new();
    for node in primary.nodes() {
        if nodes.insert(node.index(), node).is_some() {
            duplicates.insert(node.index());
        }
    }
    let mut admitted = BTreeSet::new();
    for body in nodes.values().filter(|node| node.kind() == 12) {
        if duplicates.contains(&body.index()) || body.has_base_layout() {
            continue;
        }
        if let Some(component) = named_body_component(*body, &nodes, &duplicates) {
            admitted.extend(component);
        }
    }
    admitted
        .into_iter()
        .filter_map(|index| {
            let source = nodes.get(&index)?;
            if source.has_base_layout() {
                return None;
            }
            let kind = match source.kind() {
                12 => NodeKind::Body,
                19 => NodeKind::Region,
                _ => return None,
            };
            let range = source.range();
            let shift = source.fields_start()?.checked_sub(range.start.checked_add(4)?)?;
            Some(Node { kind, xmt: NonNullXmt::try_from(index).ok()?, pos: range.start, end: range.end, shift, bytes: stream.get(range)?.to_vec() })
        })
        .collect()
}

fn named_body_component(
    body: cadmpeg_parasolid::records::primary::Node<'_>,
    nodes: &BTreeMap<u32, cadmpeg_parasolid::records::primary::Node<'_>>,
    duplicates: &BTreeSet<u32>,
) -> Option<BTreeSet<u32>> {
    body.integer("highest_node_id")?;
    if body.field_range("highest_node_id")?.start != body.fields_start()? {
        return None;
    }
    // A WORLD owner supplies an explicit body list, not a placement. Admit it
    // only after checking the complete reciprocal list and absence of instances.
    named_body_owner(body, nodes, duplicates)?;
    if body.pointer_of_class("child", 12)? != 1 {
        return None;
    }
    let body_index = body.index();
    let mut admitted = BTreeSet::from([body_index]);
    let mut regions = BTreeSet::new();
    let mut previous = 1;
    let mut current = body.pointer_of_class("region", 19)?;
    while current != 1 {
        if !regions.insert(current) || duplicates.contains(&current) {
            return None;
        }
        let region = nodes.get(&current)?;
        if region.kind() != 19
            || region.pointer_of_class("body", 12)? != body_index
            || region.pointer_of_class("previous", 19)? != previous
            || region.pointer("owner")? != 1
            || region.pointer_of_class("frame", 230)? != 1
            || !matches!(region.char("type")?, b'S' | b'V')
        {
            return None;
        }
        region.integer("node_id")?;
        if region.field_range("node_id")?.start != region.fields_start()? {
            return None;
        }
        let shells = named_shell_chain(region.pointer_of_class("shell", 13)?, nodes, duplicates, |shell| {
            shell.pointer_of_class("region", 19) == Some(current)
                && (shell.pointer_of_class("body", 12) == Some(1) || shell.pointer_of_class("body", 12) == Some(body_index))
        })?;
        let expected: BTreeSet<_> = nodes
            .values()
            .filter(|node| node.kind() == 13 && node.pointer_of_class("region", 19) == Some(current))
            .map(|node| node.index())
            .collect();
        if shells != expected {
            return None;
        }
        admitted.insert(current);
        previous = current;
        current = region.pointer_of_class("next", 19)?;
    }
    let expected: BTreeSet<_> =
        nodes.values().filter(|node| node.kind() == 19 && node.pointer_of_class("body", 12) == Some(body_index)).map(|node| node.index()).collect();
    if regions.is_empty() || regions != expected {
        return None;
    }
    let shells = named_shell_chain(body.pointer_of_class("shell", 13)?, nodes, duplicates, |shell| {
        shell.pointer_of_class("body", 12) == Some(body_index) && shell.pointer_of_class("region", 19).is_some_and(|index| regions.contains(&index))
    })?;
    let expected: BTreeSet<_> =
        nodes.values().filter(|node| node.kind() == 13 && node.pointer_of_class("body", 12) == Some(body_index)).map(|node| node.index()).collect();
    if shells.is_empty() || shells != expected {
        return None;
    }
    Some(admitted)
}

fn named_body_owner(
    body: cadmpeg_parasolid::records::primary::Node<'_>,
    nodes: &BTreeMap<u32, cadmpeg_parasolid::records::primary::Node<'_>>,
    duplicates: &BTreeSet<u32>,
) -> Option<()> {
    let owner_index = body.pointer("owner")?;
    if owner_index == 1 {
        return Some(());
    }
    if duplicates.contains(&owner_index) {
        return None;
    }
    let world = nodes.get(&owner_index)?;
    if world.kind() != 101
        || world.pointer_of_class("assembly", 10)? != 1
        || world.pointer_of_class("transform", 100)? != 1
        || !world.logical("alive")?
    {
        return None;
    }
    let mut current = world.pointer_of_class("body", 12)?;
    let mut previous = 1;
    let mut members = BTreeSet::new();
    while current != 1 {
        if !members.insert(current) || duplicates.contains(&current) {
            return None;
        }
        let member = nodes.get(&current)?;
        let owner = member.pointer_of_class("owner", 101).or_else(|| member.pointer_of_class("owner", 1040));
        if member.kind() != 12
            || owner != Some(owner_index)
            || member.pointer_of_class("previous", 12)? != previous
            || member.pointer_of_class("child", 12)? != 1
            || member.pointer_of_class("ref_instance", 11)? != 1
        {
            return None;
        }
        previous = current;
        current = member.pointer_of_class("next", 12)?;
    }
    let expected: BTreeSet<_> =
        nodes.values().filter(|node| node.kind() == 12 && node.pointer("owner") == Some(owner_index)).map(|node| node.index()).collect();
    (members.contains(&body.index()) && members == expected).then_some(())
}

fn named_shell_chain(
    mut current: u32,
    nodes: &BTreeMap<u32, cadmpeg_parasolid::records::primary::Node<'_>>,
    duplicates: &BTreeSet<u32>,
    accepts: impl Fn(cadmpeg_parasolid::records::primary::Node<'_>) -> bool,
) -> Option<BTreeSet<u32>> {
    let mut shells = BTreeSet::new();
    while current != 1 {
        if !shells.insert(current) || duplicates.contains(&current) {
            return None;
        }
        let shell = *nodes.get(&current)?;
        if shell.kind() != 13 || !shell.has_base_layout() || !accepts(shell) {
            return None;
        }
        current = shell.pointer_of_class("next", 13)?;
    }
    Some(shells)
}
