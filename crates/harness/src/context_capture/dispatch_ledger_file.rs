// SPDX-License-Identifier: MIT

//! A file-backed [`DispatchLedgerPort`] for the served prepared-dispatch receipt ledger.
//!
//! [`super::dispatch_ledger_durable`] defines the durable image and the port trait but no store.
//! This module is one concrete store an operator can attach: it keeps a single bounded JSON image
//! and replaces it atomically (a sibling temp file, a flush to disk, then a rename and a directory
//! sync), so a process that restarts reloads exactly the receipts its predecessor committed. A
//! composition that attaches no store keeps the in-session ledger; nothing here chooses a path or
//! a retention default.
//!
//! An image that cannot be read is [`DispatchLedgerError::Unavailable`], never `Ok(None)`: an
//! unknown image is not an empty one, so a restarted composition refuses a managed boundary rather
//! than assuming the write never happened. An image that is present but mis-versioned, over the
//! byte bound or malformed is [`DispatchLedgerError::Invalid`], and a reader never repairs it.

use super::dispatch_ledger_durable::{
    DispatchLedgerError, DispatchLedgerPort, DurableDispatchLedger,
};
use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// The largest durable image this port will read or write.
pub const MAX_DURABLE_DISPATCH_LEDGER_BYTES: usize = 4 * 1024 * 1024;

/// A file-backed durable store of one prepared-dispatch receipt ledger image.
///
/// The path is supplied by the owning composition. `open` performs no I/O; the file is read on
/// `load` and created or replaced on `save`.
#[derive(Clone, Debug)]
pub struct FileDispatchLedgerPort {
    path: PathBuf,
}

impl FileDispatchLedgerPort {
    /// Points the port at one image path. No file is created or read until `load`/`save`.
    #[must_use]
    pub fn open(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The image path this port reads and writes.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl DispatchLedgerPort for FileDispatchLedgerPort {
    fn load(&mut self) -> Result<Option<DurableDispatchLedger>, DispatchLedgerError> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            // No file is the honest "this composition never committed an image" answer. Every
            // other failure leaves the image unknown, so it is refused rather than treated as
            // empty.
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(DispatchLedgerError::Unavailable),
        };
        if bytes.len() > MAX_DURABLE_DISPATCH_LEDGER_BYTES {
            return Err(DispatchLedgerError::Invalid);
        }
        let image: DurableDispatchLedger =
            serde_json::from_slice(&bytes).map_err(|_| DispatchLedgerError::Invalid)?;
        image.validate()?;
        Ok(Some(image))
    }

    fn save(&mut self, ledger: &DurableDispatchLedger) -> Result<(), DispatchLedgerError> {
        // Refuse an inconsistent image before it reaches the store, so a written file is always a
        // valid image a later reader can trust.
        ledger.validate()?;
        let bytes = serde_json::to_vec(ledger).map_err(|_| DispatchLedgerError::Invalid)?;
        if bytes.len() > MAX_DURABLE_DISPATCH_LEDGER_BYTES {
            return Err(DispatchLedgerError::Invalid);
        }
        write_atomically(&self.path, &bytes).map_err(|_| DispatchLedgerError::Unavailable)
    }
}

/// Replaces one file with `bytes`, so a reader observes either the old image or the new one.
///
/// The temp file is a sibling, so the rename cannot cross a filesystem boundary. The image and its
/// directory are flushed before `save` returns: once the caller is told the receipt is durable, a
/// crash must not lose it.
fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    if let Some(parent) = parent {
        fs::create_dir_all(parent)?;
    }
    let temp = temp_path(path);
    fs::write(&temp, bytes)?;
    fs::File::open(&temp)?.sync_all()?;
    fs::rename(&temp, path)?;
    if let Some(parent) = parent {
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

/// The sibling path one atomic replacement is staged at, unique to this process.
fn temp_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map_or_else(|| OsString::from("dispatch-ledger"), ToOwned::to_owned);
    name.push(format!(".tmp-{}", std::process::id()));
    match path.parent() {
        Some(parent) => parent.join(name),
        None => PathBuf::from(name),
    }
}

#[cfg(test)]
#[path = "dispatch_ledger_file_tests.rs"]
mod tests;
