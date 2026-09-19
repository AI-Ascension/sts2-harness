// SPDX-License-Identifier: MIT

//! Durable lifecycle intent, written before any effect and replayed on restart.
//!
//! The ordering here is the whole point. A lifecycle submission records intent
//! *before* the gateway is called, so a crash between the two leaves evidence
//! that an operation may exist rather than an unaccounted effect. Replay then
//! reconciles each retained operation by its identity instead of by guessing
//! from similar state.
//!
//! The store reuses the harness's existing append-only [`OperationJournal`].
//! That journal retains the scoping key, a digest of the exact request, and one
//! outcome, but it does not retain the action kind — so this module stores the
//! intent record *beside* the journal, in the same owner-controlled directory,
//! and reconciles the two on open. A journal entry without its intent record is
//! treated as corruption rather than silently reconstructed, because an intent
//! the harness cannot read is an intent it must not act on.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::lifecycle::{
    LifecycleAction, LifecycleClassification, LifecycleFailure, LifecycleOperationState,
    LifecycleState,
};
use super::service::ManagementError;
use crate::operation_journal::{JournalDecision, OperationJournal};

#[path = "lifecycle_intent_recovery.rs"]
mod recovery;

use recovery::{
    intent_digest, journal_error, journal_key, journal_outcome, open_intent_file, reconcile,
    store_error,
};

/// Namespace prefix that keeps lifecycle attempts distinct from every other
/// journal consumer sharing an owner-controlled directory.
const JOURNAL_NAMESPACE: &str = "process-lifecycle";
/// Incarnation field for lifecycle attempts. The journal is per-directory, so
/// the value names the surface rather than a process generation.
const JOURNAL_INCARNATION: &str = "process-lifecycle-v1";
/// Journal file name inside an owner-supplied directory.
const JOURNAL_FILE_NAME: &str = "harness-process-lifecycle-journal.jsonl";
/// Intent record file name inside the same directory.
const INTENT_FILE_NAME: &str = "harness-process-lifecycle-intents.jsonl";
/// Largest accepted intent record line.
const MAX_INTENT_RECORD_BYTES: u64 = 8 * 1024;
/// Longest retained intent record count.
const MAX_INTENT_RECORDS: usize = crate::operation_journal::MAX_JOURNAL_ENTRIES;

/// One durable lifecycle intent, keyed by operation identity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleIntent {
    pub command_id: String,
    pub run_id: String,
    pub operation_id: u64,
    pub authority_epoch: u64,
    pub instance_id: String,
    pub action_kind: String,
    /// Outcome so far. `None` means the intent is durable and the effect is not
    /// yet known to have settled, so it must be reconciled by identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classification: Option<LifecycleClassification>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_state: Option<LifecycleOperationState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<LifecycleState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<LifecycleFailure>,
    /// Sequence number assigned when the intent was first recorded.
    pub sequence: u64,
}

impl LifecycleIntent {
    /// True when the outcome is still unknown and must be reconciled.
    #[must_use]
    pub fn is_unresolved(&self) -> bool {
        !matches!(
            self.classification,
            Some(
                LifecycleClassification::Accepted
                    | LifecycleClassification::Rejected
                    | LifecycleClassification::Stopped
            )
        )
    }
}

/// Durable lifecycle intent journal.
pub struct LifecycleIntentStore {
    journal: OperationJournal,
    intents: BTreeMap<u64, LifecycleIntent>,
    file: File,
    directory: PathBuf,
}

/// The gateway process-lifecycle owner supplied by the binary that holds the
/// gateway credential: the transport port together with the durable intent
/// store that survives a restart.
///
/// The two are always passed together, because a port without durable intent
/// could accept an operation it cannot reconcile after a restart.
pub type ProcessLifecycleOwner = (
    std::sync::Arc<dyn crate::management::lifecycle::ProcessLifecyclePort>,
    std::sync::Arc<std::sync::Mutex<LifecycleIntentStore>>,
);

impl LifecycleIntentStore {
    /// Opens (or creates) the store inside an owner-supplied directory.
    ///
    /// Both the journal and the intent records are replayed on open, so a
    /// restart observes every intent that was durable before the process
    /// stopped. An entry the two files disagree about fails closed.
    pub fn open(directory: impl AsRef<Path>) -> Result<Self, ManagementError> {
        let directory = directory.as_ref().to_path_buf();
        fs::create_dir_all(&directory).map_err(store_error)?;
        let journal =
            OperationJournal::open(directory.join(JOURNAL_FILE_NAME)).map_err(journal_error)?;
        let (file, records) = open_intent_file(&directory)?;
        let intents = reconcile(&journal, records)?;
        Ok(Self {
            journal,
            intents,
            file,
            directory,
        })
    }

    /// The owner-supplied directory holding both durable files.
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Records intent for one operation before any effect is attempted.
    ///
    /// Replaying an identical intent returns the retained record rather than
    /// creating a second one, so a duplicate submission cannot produce two
    /// effects. Reusing the operation identity with a different intent is a
    /// conflict and is refused.
    pub fn record_intent(
        &mut self,
        command_id: &str,
        run_id: &str,
        instance_id: &str,
        operation_id: u64,
        authority_epoch: u64,
        action: &LifecycleAction,
    ) -> Result<LifecycleIntent, ManagementError> {
        let request_digest = intent_digest(
            run_id,
            instance_id,
            operation_id,
            authority_epoch,
            action.kind(),
        );
        match self
            .journal
            .begin(journal_key(instance_id, operation_id), &request_digest)
            .map_err(journal_error)?
        {
            JournalDecision::Started(sequence) => {
                let intent = LifecycleIntent {
                    command_id: command_id.to_owned(),
                    run_id: run_id.to_owned(),
                    operation_id,
                    authority_epoch,
                    instance_id: instance_id.to_owned(),
                    action_kind: action.kind().to_owned(),
                    classification: None,
                    operation_state: None,
                    state: None,
                    failure: None,
                    sequence,
                };
                self.append(&intent)?;
                self.intents.insert(operation_id, intent.clone());
                Ok(intent)
            }
            JournalDecision::Existing(_) => {
                let retained = self.intents.get(&operation_id).cloned().ok_or_else(|| {
                    ManagementError::store(
                        "lifecycle_intent_missing",
                        "durable lifecycle intent could not be reconstructed",
                    )
                })?;
                if retained.authority_epoch != authority_epoch
                    || retained.action_kind != action.kind()
                    || retained.run_id != run_id
                {
                    return Err(ManagementError::conflict(
                        "lifecycle_operation_conflict",
                        "operation identity was already used for a different lifecycle intent",
                    ));
                }
                Ok(retained)
            }
        }
    }

    /// Records the settled outcome of a recorded intent.
    pub fn complete(
        &mut self,
        instance_id: &str,
        operation_id: u64,
        classification: LifecycleClassification,
        operation_state: LifecycleOperationState,
        state: LifecycleState,
        failure: Option<LifecycleFailure>,
    ) -> Result<LifecycleIntent, ManagementError> {
        self.journal
            .complete(
                &journal_key(instance_id, operation_id),
                journal_outcome(classification),
            )
            .map_err(journal_error)?;
        let intent = self.intents.get_mut(&operation_id).ok_or_else(|| {
            ManagementError::store(
                "lifecycle_intent_missing",
                "lifecycle outcome has no durable intent",
            )
        })?;
        intent.classification = Some(classification);
        intent.operation_state = Some(operation_state);
        intent.state = Some(state);
        intent.failure = failure;
        let updated = intent.clone();
        self.append(&updated)?;
        Ok(updated)
    }

    /// Returns the retained intent for one operation.
    #[must_use]
    pub fn intent(&self, operation_id: u64) -> Option<&LifecycleIntent> {
        self.intents.get(&operation_id)
    }

    /// Returns every retained intent, ordered by operation identity.
    #[must_use]
    pub fn intents(&self) -> &BTreeMap<u64, LifecycleIntent> {
        &self.intents
    }

    /// Returns the retained intents that still need reconciliation.
    #[must_use]
    pub fn unresolved(&self) -> Vec<LifecycleIntent> {
        self.intents
            .values()
            .filter(|intent| intent.is_unresolved())
            .cloned()
            .collect()
    }

    /// True when a stop settled for this instance before the given sequence.
    ///
    /// Stop dominance: a start admitted after a settled stop is refused rather
    /// than issued, because the operator's stop is the earlier, still-standing
    /// decision. Ordering is the durable sequence, so the fence survives a
    /// restart: the sequence of a new intent is always the highest one issued,
    /// which is why the comparison looks backwards at the retained stops.
    #[must_use]
    pub fn stop_dominates(&self, instance_id: &str, at_sequence: u64) -> bool {
        self.intents.values().any(|intent| {
            intent.instance_id == instance_id
                && intent.sequence < at_sequence
                && intent.classification == Some(LifecycleClassification::Stopped)
        })
    }

    /// The highest sequence this store has issued.
    #[must_use]
    pub fn latest_sequence(&self) -> u64 {
        self.intents
            .values()
            .map(|intent| intent.sequence)
            .max()
            .unwrap_or(0)
    }

    fn append(&mut self, intent: &LifecycleIntent) -> Result<(), ManagementError> {
        let line = serde_json::to_string(intent).map_err(|error| {
            ManagementError::store("lifecycle_intent_encode", error.to_string())
        })?;
        self.file.write_all(line.as_bytes()).map_err(store_error)?;
        self.file.write_all(b"\n").map_err(store_error)?;
        self.file.sync_all().map_err(store_error)
    }
}

/// Maximum retained lifecycle intents, matching the shared journal bound.
pub const MAX_LIFECYCLE_INTENTS: usize = crate::operation_journal::MAX_JOURNAL_ENTRIES;
