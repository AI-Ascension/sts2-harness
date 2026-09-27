// SPDX-License-Identifier: MIT

//! Read and path-safety boundary for the broker-owned provider-session metadata journal.
//!
//! These are the operations that read from, and judge, the filesystem: opening a file without
//! following a symlink, and rejecting a path that escapes its directory or has a permissive mode.
//! They are separated from the store's envelope and key handling, and from the write path in
//! `atomic_replace`, so the path-safety rules can be read, and changed, as one unit.

use super::ProviderSessionMetadataStoreError;
use std::fs::{self, File};
#[cfg(not(unix))]
use std::io;
use std::path::{Component, Path, PathBuf};

#[cfg(unix)]
pub(super) fn read_restricted_file(
    path: &Path,
) -> Result<Vec<u8>, ProviderSessionMetadataStoreError> {
    use rustix::fs::{Mode, OFlags, open};
    use std::io::Read;

    let descriptor = open(
        path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map_err(|error| {
        if error == rustix::io::Errno::NOENT {
            ProviderSessionMetadataStoreError::NotFound
        } else {
            ProviderSessionMetadataStoreError::InvalidPath
        }
    })?;
    let mut file = File::from(descriptor);
    let metadata = file
        .metadata()
        .map_err(|_| ProviderSessionMetadataStoreError::Io)?;
    if !metadata.file_type().is_file() || !restricted_file(&metadata) {
        return Err(ProviderSessionMetadataStoreError::InvalidPath);
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|_| ProviderSessionMetadataStoreError::Io)?;
    Ok(bytes)
}

#[cfg(not(unix))]
pub(super) fn read_restricted_file(
    path: &Path,
) -> Result<Vec<u8>, ProviderSessionMetadataStoreError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            ProviderSessionMetadataStoreError::NotFound
        } else {
            ProviderSessionMetadataStoreError::Io
        }
    })?;
    if !metadata.file_type().is_file() {
        return Err(ProviderSessionMetadataStoreError::InvalidPath);
    }
    fs::read(path).map_err(|_| ProviderSessionMetadataStoreError::Io)
}

pub(super) fn validate_store_path(path: &Path) -> Result<(), ProviderSessionMetadataStoreError> {
    if !path.is_absolute()
        || path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err(ProviderSessionMetadataStoreError::InvalidPath);
    }
    let Some(parent) = path.parent() else {
        return Err(ProviderSessionMetadataStoreError::InvalidPath);
    };
    if !safe_directory(parent) {
        return Err(ProviderSessionMetadataStoreError::InvalidPath);
    }
    if let Ok(metadata) = fs::symlink_metadata(path)
        && (!metadata.file_type().is_file() || !restricted_file(&metadata))
    {
        return Err(ProviderSessionMetadataStoreError::InvalidPath);
    }
    Ok(())
}

pub(super) fn safe_directory(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| {
        metadata.is_dir() && !has_symlink_component(path) && restricted_directory(&metadata)
    })
}

#[cfg(unix)]
pub(crate) fn restricted_file(metadata: &fs::Metadata) -> bool {
    use rustix::process::geteuid;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    metadata.uid() == geteuid().as_raw() && metadata.permissions().mode() & 0o077 == 0
}

#[cfg(not(unix))]
pub(crate) fn restricted_file(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(unix)]
pub(crate) fn restricted_directory(metadata: &fs::Metadata) -> bool {
    use rustix::process::geteuid;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    metadata.uid() == geteuid().as_raw() && metadata.permissions().mode() & 0o077 == 0
}

#[cfg(not(unix))]
pub(crate) fn restricted_directory(_metadata: &fs::Metadata) -> bool {
    false
}

pub(super) fn has_symlink_component(path: &Path) -> bool {
    let mut current = PathBuf::new();
    path.components().any(|component| {
        current.push(component.as_os_str());
        fs::symlink_metadata(&current).is_ok_and(|metadata| metadata.file_type().is_symlink())
    })
}
