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
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Maximum entries retained in one journal.
pub const MAX_JOURNAL_ENTRIES: usize = 4096;
/// Maximum length of a scoping field or idempotency key.
pub const MAX_JOURNAL_FIELD_BYTES: usize = 256;

/// Identity of one mutating attempt.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
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
    path: PathBuf,
    entries: BTreeMap<JournalKey, JournalEntry>,
    next_sequence: u64,
    file: Option<File>,
}

impl OperationJournal {
    /// Opens or creates a journal at `path`, replaying every durable record.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, JournalError> {
        let path = path.into();
        let mut journal = Self {
            path,
            entries: BTreeMap::new(),
            next_sequence: 1,
            file: None,
        };
        journal.replay()?;
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

    /// Returns a recorded entry, if present.
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
        let sequence = self.entries.get(key).ok_or(JournalError::Missing)?.sequence;
        let request_digest = self
            .entries
            .get(key)
            .ok_or(JournalError::Missing)?
            .request_digest
            .clone();
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

    fn replay(&mut self) -> Result<(), JournalError> {
        if !self.path.is_file() {
            return Ok(());
        }
        let reader = BufReader::new(File::open(&self.path).map_err(persistence)?);
        let lines: Vec<String> = reader
            .lines()
            .collect::<Result<Vec<_>, _>>()
            .map_err(persistence)?;
        for (index, line) in lines.iter().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<JournalEntry>(line) {
                Ok(entry) => {
                    self.next_sequence = self.next_sequence.max(entry.sequence + 1);
                    self.entries.insert(entry.key.clone(), entry);
                }
                Err(_) if index + 1 == lines.len() => break,
                Err(_) => return Err(JournalError::Corrupt),
            }
        }
        Ok(())
    }

    fn write(&mut self, entry: &JournalEntry) -> Result<(), JournalError> {
        let line = serde_json::to_string(entry).map_err(|_| JournalError::Corrupt)?;
        let file = match self.file.as_mut() {
            Some(file) => file,
            None => self
                .file
                .insert(open_append(&self.path).map_err(persistence)?),
        };
        file.write_all(line.as_bytes()).map_err(persistence)?;
        file.write_all(b"\n").map_err(persistence)?;
        file.flush().map_err(persistence)
    }
}

fn open_append(path: &Path) -> std::io::Result<File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    OpenOptions::new().create(true).append(true).open(path)
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

fn persistence(error: std::io::Error) -> JournalError {
    JournalError::Persistence(error.to_string())
}
