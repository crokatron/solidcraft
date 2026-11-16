// SPDX-License-Identifier: Apache-2.0
//! Evidence that one partition is already the latest selected snapshot.
//!
//! Siemens' April 2008 XT reference defines WORLD.current_id as the current
//! partition-mark identifier and highest_id as its highest mark identifier.
//! This checks those fields against the complete Deltas mark graph. No Deltas
//! geometry is replayed, and ambiguous or earlier snapshots remain unresolved.

use super::{Stream, StreamKind};
use cadmpeg_core::CodecError;
use cadmpeg_parasolid::deltas::pmark_state::PmarkState;
use cadmpeg_parasolid::records::deltas::{DeltasEnd, EventContent, Metadata};
use cadmpeg_parasolid::records::{DeltasRecords, PrimaryRecords};
use std::collections::{BTreeMap, BTreeSet};

/// A complete latest partition selected by matching source mark identifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CurrentPartition {
    pub(crate) partition_index: usize,
    pub(crate) mark_identifier: u32,
}

pub(crate) fn witness(streams: &[Stream]) -> Result<Option<CurrentPartition>, CodecError> {
    let mut partitions = streams.iter().enumerate().filter(|(_, s)| s.kind() == StreamKind::Partition);
    let mut deltas = streams.iter().enumerate().filter(|(_, s)| s.kind() == StreamKind::Deltas);
    let (Some((partition_index, partition)), Some((delta_index, delta))) = (partitions.next(), deltas.next()) else {
        return Ok(None);
    };
    if partitions.next().is_some()
        || deltas.next().is_some()
        || streams.iter().any(|s| s.kind() == StreamKind::Plain)
        || partition_index >= delta_index
        || partition.schema_token().is_none()
        || partition.schema_token() != delta.schema_token()
    {
        return Ok(None);
    }
    let Some(primary) = PrimaryRecords::read(&partition.inflated)? else {
        return Ok(None);
    };
    if !primary.is_complete() {
        return Ok(None);
    }
    let mut identities = BTreeSet::new();
    let mut worlds = Vec::new();
    for node in primary.nodes() {
        if !identities.insert(node.index()) {
            return Ok(None);
        }
        if node.kind() == 101 {
            worlds.push(node);
        }
    }
    let [world] = worlds.as_slice() else {
        return Ok(None);
    };
    if world.index() != 2 || world.logical("alive") != Some(true) || primary.nodes().next().map(|n| (n.kind(), n.index())) != Some((101, 2)) {
        return Ok(None);
    }
    let (Some(current), Some(highest)) = (world.integer("current_id"), world.integer("highest_id")) else {
        return Ok(None);
    };
    if current == 0 || current != highest {
        return Ok(None);
    }
    let history = DeltasRecords::read(&delta.inflated)?;
    if !matches!(history.end(), DeltasEnd::Complete) {
        return Ok(None);
    }
    let mut prefixes = history.events().iter().filter_map(|e| match e.content() {
        EventContent::PartitionPrefix(prefix) => Some(*prefix),
        _ => None,
    });
    let Some(prefix) = prefixes.next() else {
        return Ok(None);
    };
    if prefixes.next().is_some() || prefix.highest_id() != highest {
        return Ok(None);
    }
    let selected = prefix.current_pmark();
    if selected.identifier() != current
        || selected.delta_is_forward()
        || selected.first_following() != 1
        || selected.next_sibling() != 1
        || selected.prev_sibling() != 1
    {
        return Ok(None);
    }
    let mut marks = BTreeMap::from([(selected.xmt(), selected)]);
    let mut identifiers = BTreeSet::from([selected.identifier()]);
    for event in history.events() {
        if let EventContent::Metadata(Metadata::Pmark(mark)) = event.content() {
            if marks.insert(mark.xmt(), *mark).is_some() || !identifiers.insert(mark.identifier()) {
                return Ok(None);
            }
        }
    }
    if marks.contains_key(&prefix.partition_xmt()) || marks.values().any(|mark| mark.identifier() > highest) || !complete_mark_graph(&marks) {
        return Ok(None);
    }
    Ok(Some(CurrentPartition { partition_index, mark_identifier: current }))
}

fn complete_mark_graph(marks: &BTreeMap<u32, PmarkState>) -> bool {
    let mut roots = Vec::new();
    for mark in marks.values() {
        if [mark.preceding(), mark.first_following(), mark.next_sibling(), mark.prev_sibling()]
            .into_iter()
            .any(|id| id != 1 && !marks.contains_key(&id))
        {
            return false;
        }
        if mark.preceding() == 1 {
            if mark.prev_sibling() != 1 || mark.next_sibling() != 1 {
                return false;
            }
            roots.push(mark.xmt());
        } else if mark.prev_sibling() == 1 && marks.get(&mark.preceding()).map(|parent| parent.first_following()) != Some(mark.xmt()) {
            return false;
        }
        if let Some(child) = marks.get(&mark.first_following()) {
            if child.preceding() != mark.xmt() || child.prev_sibling() != 1 {
                return false;
            }
        }
        if let Some(next) = marks.get(&mark.next_sibling()) {
            if next.prev_sibling() != mark.xmt() || next.preceding() != mark.preceding() {
                return false;
            }
        }
        if let Some(previous) = marks.get(&mark.prev_sibling()) {
            if previous.next_sibling() != mark.xmt() || previous.preceding() != mark.preceding() {
                return false;
            }
        }
    }
    let [root] = roots.as_slice() else {
        return false;
    };
    let mut pending = vec![*root];
    let mut reached = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !reached.insert(id) {
            return false;
        }
        let Some(mark) = marks.get(&id) else {
            return false;
        };
        for next in [mark.first_following(), mark.next_sibling()] {
            if next != 1 {
                pending.push(next);
            }
        }
    }
    reached.len() == marks.len()
}
