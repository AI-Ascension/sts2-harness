// SPDX-License-Identifier: MIT

//! Process exclusion for a selected Running branch continuation.

use std::fs::File;
use std::path::Path;

/// Locks the durable store inode, so lexical paths, symlinks, and hard links share one owner lock.
///
/// The lock intentionally serializes resumes from the same store, including sibling branches.
/// This is conservative and prevents two harness processes from advancing one live lease.
#[cfg(unix)]
pub(super) fn acquire(
    branch_store_path: &Path,
    _experiment_id: &str,
    _branch_id: &str,
) -> Result<File, String> {
    use rustix::fs::{Mode, OFlags, open};
    use std::os::unix::fs::MetadataExt;

    let canonical_path = std::fs::canonicalize(branch_store_path)
        .map_err(|_| String::from("cannot canonicalize selected-branch store identity"))?;
    let canonical_metadata = std::fs::metadata(&canonical_path)
        .map_err(|_| String::from("cannot inspect selected-branch store identity"))?;
    if !canonical_metadata.is_file() {
        return Err(String::from(
            "selected-branch store identity is not a regular file",
        ));
    }
    let parent = canonical_path
        .parent()
        .ok_or_else(|| String::from("selected-branch store has no parent directory"))?;
    let parent_metadata = std::fs::metadata(parent)
        .map_err(|_| String::from("cannot inspect selected-branch store directory"))?;
    use std::os::unix::fs::PermissionsExt;
    if !parent_metadata.is_dir() || parent_metadata.permissions().mode() & 0o022 != 0 {
        return Err(String::from(
            "selected-branch store directory must not be group- or world-writable",
        ));
    }
    let lock = File::from(
        open(
            &canonical_path,
            OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| String::from("cannot open selected-branch store process lock"))?,
    );
    let opened_metadata = lock
        .metadata()
        .map_err(|_| String::from("cannot inspect selected-branch store process lock"))?;
    if !opened_metadata.is_file()
        || opened_metadata.dev() != canonical_metadata.dev()
        || opened_metadata.ino() != canonical_metadata.ino()
    {
        return Err(String::from(
            "selected-branch store identity changed while acquiring its process lock",
        ));
    }
    lock.try_lock().map_err(|error| match error {
        std::fs::TryLockError::WouldBlock => String::from(
            "selected branch store already has an active continuation process; resume is refused",
        ),
        std::fs::TryLockError::Error(_) => {
            String::from("cannot acquire selected-branch store process lock")
        }
    })?;
    Ok(lock)
}

#[cfg(unix)]
pub(super) fn verify(branch_store_path: &Path, lock: &File) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;

    let canonical_path = std::fs::canonicalize(branch_store_path)
        .map_err(|_| String::from("cannot recheck selected-branch store identity"))?;
    let expected = std::fs::metadata(canonical_path)
        .map_err(|_| String::from("cannot recheck selected-branch store identity"))?;
    let locked = lock
        .metadata()
        .map_err(|_| String::from("cannot inspect selected-branch store process lock"))?;
    if expected.dev() != locked.dev() || expected.ino() != locked.ino() {
        return Err(String::from(
            "selected-branch store changed while opening its durable state",
        ));
    }
    Ok(())
}

/// The current implementation has no reviewed cross-process file lock with equivalent inode
/// identity semantics on non-Unix targets, so it refuses to adopt a live Gateway owner there.
#[cfg(not(unix))]
pub(super) fn acquire(
    _branch_store_path: &Path,
    _experiment_id: &str,
    _branch_id: &str,
) -> Result<File, String> {
    Err(String::from(
        "selected-branch live-owner resume is unsupported on this platform",
    ))
}

#[cfg(not(unix))]
pub(super) fn verify(_branch_store_path: &Path, _lock: &File) -> Result<(), String> {
    Err(String::from(
        "selected-branch live-owner resume is unsupported on this platform",
    ))
}
