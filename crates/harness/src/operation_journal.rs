// SPDX-License-Identifier: MIT

//! Durable idempotency journal for mutating checkpoint operations.
//!
//! A mutating attempt is scoped to principal, instance, operation, and idempotency key. Reusing a
//! key with byte-identical input replays the recorded outcome; reusing it with different input is a
//! conflict. A lost acknowledgement is reconciled by reading the journal for the original operation
//! identity rather than by comparing similar state. The log is append-only and replayed on open, so
//! a torn final write from a crash is discarded while every earlier record survives.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::File;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

mod persistence;

/// Maximum entries retained in one journal.
pub const MAX_JOURNAL_ENTRIES: usize = 4096;
/// Maximum length of a scoping field or idempotency key.
pub const MAX_JOURNAL_FIELD_BYTES: usize = 256;

/// Identity of one mutating attempt.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JournalKey {
    /// Authenticated principal that made the request.
    pub principal: String,
    /// Runtime instance the attempt targeted.
    pub instance: String,
    /// Runtime incarnation the attempt targeted.
    pub incarnation: String,
    /// Operation type.
    pub operation: String,
    /// Caller-supplied idempotency key.
    pub idempotency_key: String,
}

impl JournalKey {
    /// Validates every scoping field.
    pub fn validate(&self) -> Result<(), JournalError> {
        for field in [
            &self.principal,
            &self.instance,
            &self.incarnation,
            &self.operation,
            &self.idempotency_key,
        ] {
            if field.is_empty() || field.len() > MAX_JOURNAL_FIELD_BYTES || field.contains('\0') {
                return Err(JournalError::InvalidKey);
            }
        }
        Ok(())
    }
}

/// Outcome recorded for one attempt.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalOutcome {
    /// The attempt was recorded but its result is not yet known.
    Pending,
    /// The operation succeeded.
    Accepted,
    /// The operation was refused.
    Rejected,
    /// The result is uncertain and must be reconciled by the same key.
    Unknown,
}

impl JournalOutcome {
    /// Returns the stable label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::Unknown => "unknown",
        }
    }
}

/// One durable journal record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct JournalEntry {
    /// Attempt identity.
    pub key: JournalKey,
    /// Digest of the exact request body.
    pub request_digest: String,
    /// Latest recorded outcome.
    pub outcome: JournalOutcome,
    /// Monotonic sequence within this journal.
    pub sequence: u64,
}

/// Result of beginning an attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JournalDecision {
    /// The key was new; the attempt may proceed.
    Started(u64),
    /// The identical attempt already exists; reconcile instead of repeating it.
    Existing(JournalEntry),
}

/// Rejection reasons for the idempotency journal.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JournalError {
    /// Another owner holds the journal's exclusive operating-system lock.
    Locked,
    /// A previous write was uncertain; reopen and reconcile before further mutations.
    Poisoned,
    /// A completion would replace a terminal outcome or reset an attempt to pending.
    InvalidTransition,
    /// A scoping field is empty, too long, or contains a NUL separator.
    InvalidKey,
    /// The request digest is not `sha256:` followed by 64 lowercase hex characters.
    InvalidDigest,
    /// The key was reused with different input.
    Conflict,
    /// The attempt was never begun.
    Missing,
    /// The journal holds [`MAX_JOURNAL_ENTRIES`].
    Capacity,
    /// An earlier record is unreadable.
    Corrupt,
    /// The backing file could not be read or written.
    Persistence(String),
}

impl fmt::Display for JournalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Locked => "journal already has an owner",
            Self::Poisoned => "journal must be reopened after an uncertain write",
            Self::InvalidTransition => "journal outcome transition is invalid",
            Self::InvalidKey => "journal key is invalid",
            Self::InvalidDigest => "journal request digest is invalid",
            Self::Conflict => "idempotency key was reused with different input",
            Self::Missing => "journal attempt is unknown",
            Self::Capacity => "journal is full",
            Self::Corrupt => "journal record is corrupt",
            Self::Persistence(_) => "journal storage failed",
        })
    }
}

impl std::error::Error for JournalError {}

/// Append-only, crash-tolerant idempotency journal.
#[derive(Debug)]
pub struct OperationJournal {
    entries: BTreeMap<JournalKey, JournalEntry>,
    next_sequence: u64,
    file: File,
    poisoned: bool,
}

impl OperationJournal {
    /// Opens or creates a journal at `path`, replaying every durable record.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, JournalError> {
        let file = persistence::open(&path.into())?;
        let mut journal = Self {
            entries: BTreeMap::new(),
            next_sequence: 1,
            file,
            poisoned: false,
        };
        persistence::replay(&mut journal)?;
        Ok(journal)
    }

    /// Returns the number of recorded attempts.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Reports whether the journal holds no attempt.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns the last acknowledged entry, if present.
    ///
    /// After an uncertain persistence error, reopen before using entries to reconcile an operation.
    #[must_use]
    pub fn entry(&self, key: &JournalKey) -> Option<&JournalEntry> {
        self.entries.get(key)
    }

    /// Begins an attempt or reports the identical existing one.
    pub fn begin(
        &mut self,
        key: JournalKey,
        request_digest: &str,
    ) -> Result<JournalDecision, JournalError> {
        self.ensure_healthy()?;
        key.validate()?;
        validate_digest(request_digest)?;
        if let Some(existing) = self.entries.get(&key) {
            return if existing.request_digest == request_digest {
                Ok(JournalDecision::Existing(existing.clone()))
            } else {
                Err(JournalError::Conflict)
            };
        }
        if self.entries.len() >= MAX_JOURNAL_ENTRIES {
            return Err(JournalError::Capacity);
        }
        let sequence = self.next_sequence;
        let entry = JournalEntry {
            key: key.clone(),
            request_digest: request_digest.to_owned(),
            outcome: JournalOutcome::Pending,
            sequence,
        };
        self.write(&entry)?;
        self.next_sequence = sequence + 1;
        self.entries.insert(key, entry);
        Ok(JournalDecision::Started(sequence))
    }

    /// Records the outcome of a begun attempt.
    pub fn complete(
        &mut self,
        key: &JournalKey,
        outcome: JournalOutcome,
    ) -> Result<(), JournalError> {
        self.ensure_healthy()?;
        let previous = self.entries.get(key).ok_or(JournalError::Missing)?;
        if previous.outcome == outcome {
            return Ok(());
        }
        if !valid_transition(previous.outcome, outcome) {
            return Err(JournalError::InvalidTransition);
        }
        let sequence = previous.sequence;
        let request_digest = previous.request_digest.clone();
        let entry = JournalEntry {
            key: key.clone(),
            request_digest,
            outcome,
            sequence,
        };
        self.write(&entry)?;
        self.entries.insert(key.clone(), entry);
        Ok(())
    }

    fn ensure_healthy(&self) -> Result<(), JournalError> {
        if self.poisoned {
            Err(JournalError::Poisoned)
        } else {
            Ok(())
        }
    }

    fn write(&mut self, entry: &JournalEntry) -> Result<(), JournalError> {
        let line = serde_json::to_string(entry).map_err(|_| JournalError::Corrupt)?;
        if let Err(error) = persistence::append(&mut self.file, &line) {
            self.poisoned = true;
            return Err(error);
        }
        Ok(())
    }
}

fn valid_transition(previous: JournalOutcome, next: JournalOutcome) -> bool {
    previous == next
        || matches!(
            (previous, next),
            (
                JournalOutcome::Pending,
                JournalOutcome::Accepted | JournalOutcome::Rejected | JournalOutcome::Unknown
            ) | (
                JournalOutcome::Unknown,
                JournalOutcome::Accepted | JournalOutcome::Rejected
            )
        )
}

fn validate_digest(value: &str) -> Result<(), JournalError> {
    let hex = value
        .strip_prefix("sha256:")
        .ok_or(JournalError::InvalidDigest)?;
    let lowercase = hex
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if hex.len() != 64 || !lowercase {
        return Err(JournalError::InvalidDigest);
    }
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod failure_tests;
