// SPDX-License-Identifier: MIT

//! Durable replacement of the broker-owned provider-session metadata journal.
//!
//! The envelope is written to a uniquely named temporary file in the destination directory,
//! given private permissions, flushed, and only then renamed over the target. A reader therefore
//! never observes a partially written journal, and a crash mid-write leaves the previous journal
//! intact. The temporary name carries the process id and a counter so two writers in one process
//! cannot collide on it.

use super::{MAX_ENVELOPE_BYTES, ProviderSessionMetadataStoreError};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use super::state_store_io::{restricted_file, safe_directory};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(1);

pub(super) fn atomic_write(
    path: &Path,
    envelope: &[u8],
) -> Result<(), ProviderSessionMetadataStoreError> {
    let parent = path
        .parent()
        .ok_or(ProviderSessionMetadataStoreError::InvalidPath)?;
    if !safe_directory(parent) || envelope.len() > MAX_ENVELOPE_BYTES {
        return Err(ProviderSessionMetadataStoreError::InvalidPath);
    }
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or(ProviderSessionMetadataStoreError::InvalidPath)?;
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(".{file_name}.tmp-{}-{counter}", std::process::id()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|_| ProviderSessionMetadataStoreError::Io)?;
    set_private_file_mode(&file).map_err(|_| ProviderSessionMetadataStoreError::Io)?;
    file.write_all(envelope)
        .map_err(|_| ProviderSessionMetadataStoreError::Io)?;
    file.sync_all()
        .map_err(|_| ProviderSessionMetadataStoreError::Io)?;
    drop(file);
    if fs::symlink_metadata(path)
        .is_ok_and(|metadata| !metadata.file_type().is_file() || !restricted_file(&metadata))
    {
        let _ = fs::remove_file(&temporary);
        return Err(ProviderSessionMetadataStoreError::InvalidPath);
    }
    fs::rename(&temporary, path).map_err(|_| {
        let _ = fs::remove_file(&temporary);
        ProviderSessionMetadataStoreError::Io
    })?;
    Ok(())
}

pub(super) fn set_private_file_mode(file: &File) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(windows)]
    {
        // There is no mode to narrow here. A file created inside the private state root inherits
        // that directory's access control, which is where the restriction lives on this platform.
        let _ = file;
    }
    Ok(())
}
