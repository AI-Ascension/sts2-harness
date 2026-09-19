// SPDX-License-Identifier: MIT

//! Durable replay, cross-check, and journal binding for lifecycle intent.
//!
//! Replay is separated from the store only to keep each module inside the
//! repository size bound; the ordering rule is unchanged. The intent records
//! and the shared operation journal must agree, a torn final line is discarded
//! rather than guessed at, and any disagreement fails closed instead of being
//! reconstructed.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use crate::management::ManagementError;
use crate::management::lifecycle::LifecycleClassification;
use crate::operation_journal::{JournalError, JournalKey, JournalOutcome, OperationJournal};

use super::{
    INTENT_FILE_NAME, JOURNAL_INCARNATION, JOURNAL_NAMESPACE, LifecycleIntent,
    MAX_INTENT_RECORD_BYTES, MAX_INTENT_RECORDS,
};

/// Opens the intent-record file and replays it, discarding a torn final line.
pub(super) fn open_intent_file(
    directory: &Path,
) -> Result<(File, Vec<LifecycleIntent>), ManagementError> {
    let path = directory.join(INTENT_FILE_NAME);
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(store_error)?;
    let mut reader = BufReader::new(file.try_clone().map_err(store_error)?);
    let mut records = Vec::new();
    let mut committed = 0_u64;
    loop {
        let mut bytes = Vec::new();
        reader
            .by_ref()
            .take(MAX_INTENT_RECORD_BYTES + 1)
            .read_until(b'\n', &mut bytes)
            .map_err(store_error)?;
        if bytes.is_empty() {
            break;
        }
        if bytes.len() as u64 > MAX_INTENT_RECORD_BYTES {
            return Err(ManagementError::store(
                "lifecycle_intent_oversized",
                "durable lifecycle intent record exceeds its bound",
            ));
        }
        if bytes.last() != Some(&b'\n') {
            // A torn final write from a crash; discard it and truncate.
            file.set_len(committed).map_err(store_error)?;
            file.sync_all().map_err(store_error)?;
            break;
        }
        if records.len() >= MAX_INTENT_RECORDS {
            return Err(ManagementError::store(
                "lifecycle_intent_capacity",
                "durable lifecycle intent store is full",
            ));
        }
        let intent: LifecycleIntent = serde_json::from_slice(&bytes).map_err(|_| {
            ManagementError::store(
                "lifecycle_intent_corrupt",
                "durable lifecycle intent record is unreadable",
            )
        })?;
        records.push(intent);
        committed += bytes.len() as u64;
    }
    Ok((file, records))
}

/// Rebuilds the latest intent per operation and cross-checks the journal.
///
/// The two durable files must agree. A journal entry without an intent record
/// means the harness cannot know what it asked for, and an intent record
/// without a journal entry means an effect may exist with no recorded
/// authority to reconcile it. Both fail closed rather than being guessed.
pub(super) fn reconcile(
    journal: &OperationJournal,
    records: Vec<LifecycleIntent>,
) -> Result<BTreeMap<u64, LifecycleIntent>, ManagementError> {
    let mut intents: BTreeMap<u64, LifecycleIntent> = BTreeMap::new();
    for record in records {
        let existing = intents.get(&record.operation_id);
        match existing {
            None => {
                // The journal assigns one monotonic sequence per journal file,
                // so a later operation's first record legitimately starts above
                // one. The journal entry itself is the authority for that
                // value, and the cross-check below compares the two.
                if record.sequence == 0 {
                    return Err(ManagementError::store(
                        "lifecycle_intent_corrupt",
                        "durable lifecycle intent has no journal sequence",
                    ));
                }
                intents.insert(record.operation_id, record);
            }
            Some(previous) => {
                if previous.sequence != record.sequence
                    || previous.command_id != record.command_id
                    || previous.run_id != record.run_id
                    || previous.instance_id != record.instance_id
                    || previous.authority_epoch != record.authority_epoch
                    || previous.action_kind != record.action_kind
                    || !outcome_transition_allowed(previous.classification, record.classification)
                {
                    return Err(ManagementError::store(
                        "lifecycle_intent_corrupt",
                        "durable lifecycle intent history is inconsistent",
                    ));
                }
                intents.insert(record.operation_id, record);
            }
        }
    }
    for (operation_id, intent) in &intents {
        let entry = journal
            .entry(&journal_key(&intent.instance_id, *operation_id))
            .ok_or_else(|| {
                ManagementError::store(
                    "lifecycle_intent_corrupt",
                    "durable lifecycle intent has no journal attempt",
                )
            })?;
        if entry.sequence != intent.sequence
            || entry.request_digest
                != intent_digest(
                    &intent.run_id,
                    &intent.instance_id,
                    *operation_id,
                    intent.authority_epoch,
                    &intent.action_kind,
                )
        {
            return Err(ManagementError::store(
                "lifecycle_intent_corrupt",
                "durable lifecycle intent disagrees with its journal attempt",
            ));
        }
        // The journal retains a coarser outcome than the intent's
        // classification (`Stopped` is journalled as accepted and `Unknown` as
        // its own outcome), so the two are compared by mapping the intent's
        // classification onto the journal's domain rather than the reverse.
        let expected = intent
            .classification
            .map(journal_outcome)
            .unwrap_or(JournalOutcome::Pending);
        if expected != entry.outcome {
            return Err(ManagementError::store(
                "lifecycle_intent_corrupt",
                "durable lifecycle intent outcome disagrees with its journal attempt",
            ));
        }
    }
    if journal.len() != intents.len() {
        return Err(ManagementError::store(
            "lifecycle_intent_corrupt",
            "durable lifecycle journal holds an attempt with no intent record",
        ));
    }
    Ok(intents)
}

fn outcome_transition_allowed(
    previous: Option<LifecycleClassification>,
    next: Option<LifecycleClassification>,
) -> bool {
    let from = previous
        .map(journal_outcome)
        .unwrap_or(JournalOutcome::Pending);
    let to = next.map(journal_outcome).unwrap_or(JournalOutcome::Pending);
    from == to
        || matches!(
            (from, to),
            (
                JournalOutcome::Pending,
                JournalOutcome::Accepted | JournalOutcome::Rejected | JournalOutcome::Unknown
            ) | (
                JournalOutcome::Unknown,
                JournalOutcome::Accepted | JournalOutcome::Rejected
            )
        )
}

/// Builds the journal key for one lifecycle operation.
pub(super) fn journal_key(instance_id: &str, operation_id: u64) -> JournalKey {
    JournalKey {
        principal: JOURNAL_NAMESPACE.to_owned(),
        instance: instance_id.to_owned(),
        incarnation: JOURNAL_INCARNATION.to_owned(),
        operation: format!("op:{operation_id}"),
        idempotency_key: format!("op-{operation_id}"),
    }
}

/// Digest binding the intent facts that must not change on replay.
///
/// The action kind is embedded so a reused operation identity with a different
/// action is detected as a conflict by the journal itself. This is a
/// journal-internal integrity digest over bounded fields, not a wire digest.
pub(super) fn intent_digest(
    run_id: &str,
    instance_id: &str,
    operation_id: u64,
    authority_epoch: u64,
    action_kind: &str,
) -> String {
    let material = format!(
        "{run_id}\u{1f}{instance_id}\u{1f}{operation_id}\u{1f}{authority_epoch}\u{1f}{action_kind}"
    );
    format!("sha256:{}", crate::sha256_hex(material.as_bytes()))
}

pub(super) fn journal_outcome(classification: LifecycleClassification) -> JournalOutcome {
    match classification {
        LifecycleClassification::Accepted | LifecycleClassification::Stopped => {
            JournalOutcome::Accepted
        }
        LifecycleClassification::Rejected => JournalOutcome::Rejected,
        LifecycleClassification::Unknown => JournalOutcome::Unknown,
    }
}

pub(super) fn journal_error(error: JournalError) -> ManagementError {
    ManagementError::store("lifecycle_journal_unavailable", error.to_string())
}

pub(super) fn store_error(error: std::io::Error) -> ManagementError {
    ManagementError::store("lifecycle_intent_store", error.to_string())
}
