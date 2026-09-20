// SPDX-License-Identifier: MIT

//! The durable, append-only retained history.
//!
//! The store keeps one history per run/branch/episode/epoch, records every append under its
//! operation identity so a re-delivered batch is a no-op, and copies an ancestor's history to a fork
//! by lineage so a replayed or re-joined batch cannot duplicate an event. It reaches no host, no
//! game process and no gateway: it is a harness-owned historical artifact.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::error::{
    SemanticHistoryError, SemanticHistoryRefusal as Refusal, SemanticHistoryResult,
};
use super::ingest::admit_batch;
use super::record::SemanticEventRecord;
use super::replay::{
    SemanticAppendOutcome, SemanticForkOutcome, SemanticHistoryAppend, SemanticHistoryFork,
    validate_fork,
};
use super::retention::{SemanticPrunePlan, merge_intervals};
use super::scope::{
    SEMANTIC_MAX_EVENTS, SEMANTIC_MAX_HISTORY_BYTES, SemanticCatalogBinding, SemanticEventScope,
};

mod backfill;
mod file;
mod prune;

/// One retained history: its scope, its binding, and its records in sequence order.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct RetainedHistory {
    binding: SemanticCatalogBinding,
    scope: SemanticEventScope,
    window: super::record::SemanticCaptureWindow,
    records: Vec<SemanticEventRecord>,
    parent_branch_id: Option<String>,
    operation_ids: BTreeMap<String, String>,
    #[serde(default)]
    retention_intervals: Vec<super::record::SemanticCoverageInterval>,
    #[serde(default)]
    prune_plans: BTreeMap<String, SemanticPrunePlan>,
}

/// The whole retained store, keyed by branch identity.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct RetainedStore {
    version: u32,
    histories: BTreeMap<String, RetainedHistory>,
}

const STORE_VERSION: u32 = 1;

/// A durable history store rooted at one private file.
#[derive(Clone, Debug)]
pub struct SemanticHistoryStore {
    path: PathBuf,
    state: RetainedStore,
}

impl SemanticHistoryStore {
    /// Opens the store at `path`, replaying whatever it already holds.
    pub fn open(path: PathBuf) -> SemanticHistoryResult<Self> {
        let state = file::load(&path)?;
        if state.version != STORE_VERSION {
            return Err(SemanticHistoryError::new(Refusal::Storage));
        }
        Ok(Self { path, state })
    }

    /// Opens a store over an empty state without touching the filesystem.
    #[must_use]
    pub fn in_memory(path: PathBuf) -> Self {
        Self {
            path,
            state: RetainedStore {
                version: STORE_VERSION,
                histories: BTreeMap::new(),
            },
        }
    }

    /// Appends one batch, idempotently by operation identity.
    pub fn append(
        &mut self,
        append: &SemanticHistoryAppend,
    ) -> SemanticHistoryResult<SemanticAppendOutcome> {
        let records = admit_batch(&append.binding, &append.batch)?;
        let key = append.batch.scope.branch_id.clone();
        let payload = file::digest_append(append)?;
        if let Some(existing) = self.state.histories.get(&key) {
            if let Some(previous) = existing.operation_ids.get(&append.operation_id) {
                return if *previous == payload {
                    Ok(SemanticAppendOutcome::AlreadyPresent {
                        total: existing.records.len(),
                    })
                } else {
                    Err(SemanticHistoryError::about(
                        Refusal::IdempotencyConflict,
                        &append.operation_id,
                    ))
                };
            }
            if existing.binding != append.binding {
                return Err(SemanticHistoryError::new(Refusal::BindingMismatch));
            }
            if existing.scope != append.batch.scope {
                return Err(SemanticHistoryError::new(Refusal::ScopeMismatch));
            }
            let expected = existing
                .records
                .last()
                .map_or(append.batch.window.capture_start_sequence, |record| {
                    record.event.sequence.saturating_add(1)
                });
            if append.batch.window.capture_start_sequence != expected {
                return Err(SemanticHistoryError::new(Refusal::NotContiguous));
            }
        }
        let added = records.len();
        // Every fallible step happens before the retained state is touched, so a refused append
        // leaves the history exactly as it was rather than removing what it could not extend.
        //
        // A history keeps where its capture began, and the spans it declared, from the batch that
        // created it: a later batch states a continuation, so it adds its own spans to the retained
        // list rather than restating its own origin or replacing what an earlier batch declared.
        let retained = self.state.histories.get(&key);
        let origin = retained.map_or(
            (
                append.batch.window.capture_start_sequence,
                append.batch.window.history_before_capture,
            ),
            |history| {
                (
                    history.window.capture_start_sequence,
                    history.window.history_before_capture,
                )
            },
        );
        let declared = retained.map_or_else(Vec::new, |history| history.window.intervals.clone());
        let window = super::record::SemanticCaptureWindow {
            capture_start_sequence: origin.0,
            history_before_capture: origin.1,
            intervals: merge_intervals(&declared, &append.batch.window.intervals)?,
        };
        let mut history = self
            .state
            .histories
            .remove(&key)
            .unwrap_or(RetainedHistory {
                binding: append.binding.clone(),
                scope: append.batch.scope.clone(),
                window: super::record::SemanticCaptureWindow {
                    capture_start_sequence: origin.0,
                    history_before_capture: origin.1,
                    intervals: Vec::new(),
                },
                records: Vec::new(),
                parent_branch_id: None,
                operation_ids: BTreeMap::new(),
                retention_intervals: Vec::new(),
                prune_plans: BTreeMap::new(),
            });
        history.records.extend(records);
        history.window = window;
        history
            .operation_ids
            .insert(append.operation_id.clone(), payload);
        let total = history.records.len();
        self.commit(key, history)?;
        Ok(SemanticAppendOutcome::Appended { added, total })
    }

    /// Copies an ancestor's retained history to a child branch, once per fork identity.
    pub fn fork(
        &mut self,
        fork: &SemanticHistoryFork,
    ) -> SemanticHistoryResult<SemanticForkOutcome> {
        validate_fork(fork)?;
        let payload = file::digest_fork(fork)?;
        if let Some(existing) = self.state.histories.get(&fork.child.branch_id)
            && let Some(previous) = existing.operation_ids.get(&fork.operation_id)
        {
            return if *previous == payload {
                Ok(SemanticForkOutcome {
                    inherited: existing.records.len(),
                    total: existing.records.len(),
                    already_present: true,
                })
            } else {
                Err(SemanticHistoryError::about(
                    Refusal::IdempotencyConflict,
                    &fork.operation_id,
                ))
            };
        }
        let parent = self
            .state
            .histories
            .get(&fork.parent_branch_id)
            .cloned()
            .ok_or_else(|| SemanticHistoryError::new(Refusal::UnknownParentBranch))?;
        let inherited = parent.records.len();
        let mut history = RetainedHistory {
            binding: parent.binding.clone(),
            scope: fork.child.clone(),
            window: parent.window.clone(),
            records: parent.records,
            parent_branch_id: Some(fork.parent_branch_id.clone()),
            operation_ids: BTreeMap::new(),
            retention_intervals: parent.retention_intervals,
            prune_plans: BTreeMap::new(),
        };
        history
            .operation_ids
            .insert(fork.operation_id.clone(), payload);
        let key = fork.child.branch_id.clone();
        self.commit(key, history)?;
        Ok(SemanticForkOutcome {
            inherited,
            total: inherited,
            already_present: false,
        })
    }

    /// Returns the retained records of one branch, or `None` when it is not retained.
    #[must_use]
    pub fn records(&self, branch_id: &str) -> Option<&[SemanticEventRecord]> {
        self.state
            .histories
            .get(branch_id)
            .map(|history| history.records.as_slice())
    }

    /// Returns the retained binding of one branch.
    #[must_use]
    pub fn binding(&self, branch_id: &str) -> Option<&SemanticCatalogBinding> {
        self.state
            .histories
            .get(branch_id)
            .map(|history| &history.binding)
    }

    /// Returns the retained scope of one branch.
    #[must_use]
    pub fn scope(&self, branch_id: &str) -> Option<&SemanticEventScope> {
        self.state
            .histories
            .get(branch_id)
            .map(|history| &history.scope)
    }

    /// Returns the retained capture window of one branch.
    #[must_use]
    pub fn window(&self, branch_id: &str) -> Option<&super::record::SemanticCaptureWindow> {
        self.state
            .histories
            .get(branch_id)
            .map(|history| &history.window)
    }

    /// Returns the branch this one was forked from, when it is a fork.
    #[must_use]
    pub fn parent_branch(&self, branch_id: &str) -> Option<&str> {
        self.state
            .histories
            .get(branch_id)
            .and_then(|history| history.parent_branch_id.as_deref())
    }

    /// Returns the number of branches the store retains.
    #[must_use]
    pub fn branch_count(&self) -> usize {
        self.state.histories.len()
    }

    fn commit(&mut self, key: String, history: RetainedHistory) -> SemanticHistoryResult<()> {
        if history.records.len() > SEMANTIC_MAX_EVENTS {
            return Err(SemanticHistoryError::new(Refusal::TooManyEvents));
        }
        let bytes = history
            .records
            .iter()
            .map(|record| record.event.byte_len())
            .sum::<usize>();
        if bytes > SEMANTIC_MAX_HISTORY_BYTES {
            return Err(SemanticHistoryError::new(Refusal::TooManyBytes));
        }
        // The write is the last step and it is undone if it fails, so a commit that cannot reach the
        // file leaves the in-memory history as the file still describes it.
        let previous = self.state.histories.insert(key.clone(), history);
        match file::store(&self.path, &self.state) {
            Ok(()) => Ok(()),
            Err(error) => {
                match previous {
                    Some(previous) => {
                        self.state.histories.insert(key, previous);
                    }
                    None => {
                        self.state.histories.remove(&key);
                    }
                }
                Err(error)
            }
        }
    }
}
