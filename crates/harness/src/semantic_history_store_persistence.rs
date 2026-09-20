// SPDX-License-Identifier: MIT

//! Encoding one run's history, and validated restoration of it.
//!
//! A history that only exists inside one process cannot survive the restart it claims to survive.
//! This module writes the owner scope, the branch lineage, the capture window and every stored event
//! as one canonical document, and reads it back by re-deriving every rule the store enforces at
//! append time.
//!
//! Restoration is a validation rather than a cast. A record that could not have been appended is
//! refused instead of loaded, so a truncated or edited document cannot introduce an event the
//! boundary would have rejected, a renumbered sequence, a closed gap, or a causal link that never
//! held. What is restored is therefore the same history the writer had, or no history at all.

use super::{BranchHistory, SemanticHistoryStore, event_digest};
use crate::semantic_history::{
    Error, MAX_HISTORY_BRANCH_DEPTH, SEMANTIC_HISTORY_SCHEMA, SemanticHistoryBinding,
    SemanticHistoryCaptureWindow, SemanticHistoryEvent, SemanticHistoryLineage, validation,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The encoded document: one owner scope and every branch it owns.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    /// The schema the document was written under.
    schema: String,
    /// The owner scope every record in it belongs to.
    binding: SemanticHistoryBinding,
    /// The capture window, including every declared gap.
    window: SemanticHistoryCaptureWindow,
    /// The branch edges.
    lineage: Vec<SemanticHistoryLineage>,
    /// The branches and their events, in stored order.
    branches: Vec<WireBranch>,
}

/// One branch and the events recorded on it.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireBranch {
    /// The branch these events were appended to.
    branch_id: String,
    /// The events, oldest first.
    events: Vec<SemanticHistoryEvent>,
}

pub(super) fn encode(store: &SemanticHistoryStore) -> Result<Vec<u8>, Error> {
    let document = Document {
        schema: SEMANTIC_HISTORY_SCHEMA.to_owned(),
        binding: store.binding.clone(),
        window: store.window.clone(),
        lineage: store.lineage.clone(),
        branches: store
            .branches
            .iter()
            .map(|(branch_id, branch)| WireBranch {
                branch_id: branch_id.clone(),
                events: branch.events.clone(),
            })
            .collect(),
    };
    serde_json::to_vec(&document).map_err(|_| Error::Corrupt)
}

pub(super) fn restore(bytes: &[u8]) -> Result<SemanticHistoryStore, Error> {
    let document: Document = serde_json::from_slice(bytes).map_err(|_| Error::Corrupt)?;
    if document.schema != SEMANTIC_HISTORY_SCHEMA {
        // A document written under another schema is not this history.
        return Err(Error::Corrupt);
    }
    document.binding.validate()?;
    document.window.validate()?;
    let mut branches = BTreeMap::new();
    for branch in &document.branches {
        if branches.contains_key(&branch.branch_id) {
            return Err(Error::Branch);
        }
        let mut restored = BranchHistory::new();
        for (position, event) in branch.events.iter().enumerate() {
            let previous = position.checked_sub(1).map(|index| &branch.events[index]);
            restore_event(event, previous, branch, &document, &mut restored)?;
        }
        // An epoch advance resets host sequencing without discarding what was already recorded, so
        // the branch is left expecting the next sequence only if it already holds an event in the
        // epoch this store now serves. Otherwise it is exactly as `advance_epoch` left it.
        if let Some(last) = restored.events.last()
            && last.input.authority_epoch == document.binding.authority_epoch
        {
            restored.last_sequence = Some(last.input.sequence);
        }
        branches.insert(branch.branch_id.clone(), restored);
    }
    let lineage = restore_lineage(&document)?;
    Ok(SemanticHistoryStore {
        binding: document.binding,
        lineage,
        branches,
        window: document.window,
    })
}

/// Restores one event, re-deriving every rule the append path applies to it.
fn restore_event(
    event: &SemanticHistoryEvent,
    previous: Option<&SemanticHistoryEvent>,
    branch: &WireBranch,
    document: &Document,
    restored: &mut BranchHistory,
) -> Result<(), Error> {
    if event.schema != SEMANTIC_HISTORY_SCHEMA || event.branch_id != branch.branch_id {
        return Err(Error::Corrupt);
    }
    validation::validate_input(
        &event.input,
        &event.causal_parent,
        &branch.branch_id,
        &document.binding.run_id,
    )?;
    if event.causal_parent.is_stated() && !event.input.origin.admits_stated_parent() {
        return Err(Error::ImportedStatesParent);
    }
    if event.input.authority_epoch > document.binding.authority_epoch {
        // An epoch this store never reached cannot have recorded an event.
        return Err(Error::Epoch);
    }
    if document.window.is_before_capture(event.input.sequence) {
        return Err(Error::Coverage);
    }
    if let Some(gap) = document.window.gap_at(event.input.sequence) {
        // A captured event inside a declared gap would close a gap the capture said it could not
        // observe, so it is refused rather than restored.
        if event.input.coverage.status.is_observed() || gap.status != event.input.coverage.status {
            return Err(Error::Coverage);
        }
    }
    if restored.by_id.contains_key(&event.input.event_id) {
        return Err(Error::MixedGeneration);
    }
    match previous {
        None => (),
        Some(previous) if previous.input.authority_epoch > event.input.authority_epoch => {
            // Epochs only advance, so a later record cannot belong to an earlier one.
            return Err(Error::Epoch);
        }
        Some(previous) if previous.input.authority_epoch == event.input.authority_epoch => {
            if event.input.sequence <= previous.input.sequence {
                return Err(Error::Sequence);
            }
            if event.input.sequence > previous.input.sequence + 1 {
                let declared = document
                    .window
                    .gap_at(previous.input.sequence + 1)
                    .is_some_and(|gap| gap.to_sequence >= event.input.sequence - 1);
                if !declared {
                    return Err(Error::Sequence);
                }
            }
        }
        // A new epoch may restart host sequencing, so the sequence is unconstrained here.
        Some(_) => (),
    }
    if let Some(parent) = event.causal_parent.stated_event_id() {
        let parent_index = restored
            .by_id
            .get(parent)
            .copied()
            .ok_or(Error::Causality)?;
        if restored.events[parent_index].input.sequence >= event.input.sequence {
            return Err(Error::ParentNotPreceding);
        }
        if restored.events[parent_index].input.authority_epoch != event.input.authority_epoch {
            return Err(Error::Causality);
        }
    }
    // The digest is recomputed rather than trusted, so a document whose content and digest disagree
    // is refused instead of loaded as a record this store would never have written. This also
    // re-applies the per-event size bound, because computing the digest refuses an oversized one.
    if event_digest(&event.input, &event.causal_parent)? != event.content_digest {
        return Err(Error::Corrupt);
    }
    let index = restored.events.len();
    restored.by_id.insert(event.input.event_id.clone(), index);
    restored.events.push(event.clone());
    Ok(())
}

/// Restores the lineage, refusing a document whose edges and branches disagree.
///
/// The ancestry walk is bounded, so a chain deeper than the bound and a chain that closes on itself
/// are refused by the same rule: an edge whose ancestry has no end cannot order a run's histories.
fn restore_lineage(document: &Document) -> Result<Vec<SemanticHistoryLineage>, Error> {
    let mut lineage: Vec<SemanticHistoryLineage> = Vec::with_capacity(document.lineage.len());
    let mut roots = 0_usize;
    for edge in &document.lineage {
        edge.validate()?;
        let known = document
            .branches
            .iter()
            .any(|branch| branch.branch_id == edge.branch_id);
        if !known {
            // An edge with no branch, or a branch with no edge, would leave the lineage and the
            // content disagreeing about which scopes exist.
            return Err(Error::Lineage);
        }
        if edge.authority_epoch > document.binding.authority_epoch {
            return Err(Error::Epoch);
        }
        if edge.parent_branch_id.is_none() {
            roots += 1;
        }
        lineage.push(edge.clone());
    }
    if lineage.len() != document.branches.len() || roots != 1 {
        // Every branch is reached from exactly one root; a second root would leave the run with two
        // histories that cannot be ordered against each other.
        return Err(Error::Lineage);
    }
    for edge in &lineage {
        let mut depth = 1_usize;
        let mut current = edge.parent_branch_id.clone();
        while let Some(branch) = current {
            depth += 1;
            if depth > MAX_HISTORY_BRANCH_DEPTH {
                return Err(Error::Lineage);
            }
            current = lineage
                .iter()
                .find(|candidate| candidate.branch_id == branch)
                .and_then(|candidate| candidate.parent_branch_id.clone());
        }
    }
    Ok(lineage)
}
