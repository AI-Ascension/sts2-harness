// SPDX-License-Identifier: MIT

//! Exclusive, synchronized journal records and bounded crash recovery.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;

use super::{JournalEntry, JournalError, JournalOutcome, MAX_JOURNAL_ENTRIES, OperationJournal};

const MAX_RECORD_BYTES: u64 = 16 * 1024;
const MAX_RECORDS: usize = MAX_JOURNAL_ENTRIES * 3;

pub(super) fn open(path: &Path) -> Result<File, JournalError> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    create_directories(parent)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(error)?;
    file.try_lock().map_err(|failure| match failure {
        TryLockError::WouldBlock => JournalError::Locked,
        TryLockError::Error(failure) => error(failure),
    })?;
    file.sync_all().map_err(error)?;
    sync_directory(parent)?;
    Ok(file)
}

fn create_directories(path: &Path) -> Result<(), JournalError> {
    if path.is_dir() {
        return Ok(());
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    create_directories(parent)?;
    match fs::create_dir(path) {
        Ok(()) => (),
        Err(failure) if failure.kind() == std::io::ErrorKind::AlreadyExists && path.is_dir() => (),
        Err(failure) => return Err(error(failure)),
    }
    sync_directory(parent)
}

fn sync_directory(path: &Path) -> Result<(), JournalError> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(error)
}

pub(super) fn replay(journal: &mut OperationJournal) -> Result<(), JournalError> {
    let mut reader = BufReader::new(journal.file.try_clone().map_err(error)?);
    let mut committed = 0_u64;
    let mut records = 0_usize;
    loop {
        let mut bytes = Vec::new();
        reader
            .by_ref()
            .take(MAX_RECORD_BYTES + 1)
            .read_until(b'\n', &mut bytes)
            .map_err(error)?;
        if bytes.is_empty() {
            break;
        }
        if bytes.len() as u64 > MAX_RECORD_BYTES {
            return Err(JournalError::Capacity);
        }
        if bytes.last() != Some(&b'\n') {
            journal.file.set_len(committed).map_err(error)?;
            journal.file.sync_all().map_err(error)?;
            break;
        }
        records += 1;
        if records > MAX_RECORDS {
            return Err(JournalError::Capacity);
        }
        let entry: JournalEntry =
            serde_json::from_slice(&bytes).map_err(|_| JournalError::Corrupt)?;
        apply_record(journal, entry)?;
        committed += bytes.len() as u64;
    }
    journal.file.seek(SeekFrom::End(0)).map_err(error)?;
    Ok(())
}

fn apply_record(journal: &mut OperationJournal, entry: JournalEntry) -> Result<(), JournalError> {
    entry.key.validate().map_err(|_| JournalError::Corrupt)?;
    super::validate_digest(&entry.request_digest).map_err(|_| JournalError::Corrupt)?;
    match journal.entries.get(&entry.key) {
        Some(previous) => {
            if previous.sequence != entry.sequence
                || previous.request_digest != entry.request_digest
                || !super::valid_transition(previous.outcome, entry.outcome)
            {
                return Err(JournalError::Corrupt);
            }
        }
        None => {
            if journal.entries.len() >= MAX_JOURNAL_ENTRIES {
                return Err(JournalError::Capacity);
            }
            if entry.sequence != journal.next_sequence || entry.outcome != JournalOutcome::Pending {
                return Err(JournalError::Corrupt);
            }
            journal.next_sequence = entry.sequence.checked_add(1).ok_or(JournalError::Corrupt)?;
        }
    }
    journal.entries.insert(entry.key.clone(), entry);
    Ok(())
}

pub(super) fn append(file: &mut File, line: &str) -> Result<(), JournalError> {
    file.write_all(line.as_bytes()).map_err(error)?;
    file.write_all(b"\n").map_err(error)?;
    file.sync_all().map_err(error)
}

fn error(failure: std::io::Error) -> JournalError {
    JournalError::Persistence(failure.to_string())
}
