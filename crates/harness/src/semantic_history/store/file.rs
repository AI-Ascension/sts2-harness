// SPDX-License-Identifier: MIT

//! Durable bytes for the retained history: one versioned JSON document written by atomic rename.
//!
//! Every write lands in a sibling temporary file and is renamed over the store, so a crash leaves
//! the previous document intact instead of a half-written one. A document that cannot be read, is
//! over its byte bound, or does not parse is refused rather than reopened as an empty history,
//! because an empty history is indistinguishable from a lost one. A missing file is the one case
//! that legitimately opens empty, and a re-delivered batch is reconciled by identity after that.

use std::fs;
use std::path::Path;

use super::super::error::{
    SemanticHistoryError, SemanticHistoryRefusal as Refusal, SemanticHistoryResult,
};
use super::super::replay::{SemanticHistoryAppend, SemanticHistoryFork};
use super::RetainedStore;

const MAX_STORE_BYTES: u64 = 16 * 1024 * 1024;

/// Reads the retained store, discarding an unparseable tail.
pub(super) fn load(path: &Path) -> SemanticHistoryResult<RetainedStore> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RetainedStore {
                version: 1,
                histories: Default::default(),
            });
        }
        Err(_) => return Err(SemanticHistoryError::new(Refusal::Storage)),
    };
    if bytes.len() as u64 > MAX_STORE_BYTES {
        return Err(SemanticHistoryError::new(Refusal::Storage));
    }
    serde_json::from_slice(&bytes).map_err(|_| SemanticHistoryError::new(Refusal::Storage))
}

/// Writes the retained store through a temporary file and an atomic rename.
pub(super) fn store(path: &Path, state: &RetainedStore) -> SemanticHistoryResult<()> {
    let bytes =
        serde_json::to_vec(state).map_err(|_| SemanticHistoryError::new(Refusal::Storage))?;
    if bytes.len() as u64 > MAX_STORE_BYTES {
        return Err(SemanticHistoryError::new(Refusal::TooManyBytes));
    }
    let temporary = path.with_extension("partial");
    fs::write(&temporary, &bytes).map_err(|_| SemanticHistoryError::new(Refusal::Storage))?;
    fs::rename(&temporary, path).map_err(|_| SemanticHistoryError::new(Refusal::Storage))
}

/// A stable digest of one append request, used to tell a replay from a conflicting reuse.
pub(super) fn digest_append(append: &SemanticHistoryAppend) -> SemanticHistoryResult<String> {
    let bytes =
        serde_json::to_vec(append).map_err(|_| SemanticHistoryError::new(Refusal::Storage))?;
    Ok(crate::sha256_hex(bytes))
}

/// A stable digest of one fork request.
pub(super) fn digest_fork(fork: &SemanticHistoryFork) -> SemanticHistoryResult<String> {
    let bytes =
        serde_json::to_vec(fork).map_err(|_| SemanticHistoryError::new(Refusal::Storage))?;
    Ok(crate::sha256_hex(bytes))
}
