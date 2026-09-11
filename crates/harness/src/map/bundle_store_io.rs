// SPDX-License-Identifier: MIT

use super::bundle::{MapBundleError, MapViewBundle};
use super::bundle_store::{MAP_MAX_FEED_ENTRIES, MAP_MAX_STORE_SCAN_ENTRIES};
use super::bundle_validation::is_digest;
use super::feed;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::Path;
use std::thread;
use std::time::Duration;

const PUBLICATION_LOCK_FILE: &str = ".sts2-map-publication.lock";
const PUBLICATION_LOCK_ATTEMPTS: usize = 32;
const PUBLICATION_LOCK_RETRY: Duration = Duration::from_millis(5);

pub(crate) struct PublicationLock {
    _file: File,
}

pub(crate) fn acquire_publication_lock(root: &Path) -> Result<PublicationLock, MapBundleError> {
    let path = root.join(PUBLICATION_LOCK_FILE);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(MapBundleError::Storage(
                "publication lock is not a regular file".to_owned(),
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(storage_error(error)),
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(storage_error)?;
    for _ in 0..PUBLICATION_LOCK_ATTEMPTS {
        match file.try_lock() {
            Ok(()) => return Ok(PublicationLock { _file: file }),
            Err(TryLockError::WouldBlock) => thread::sleep(PUBLICATION_LOCK_RETRY),
            Err(TryLockError::Error(error)) => return Err(storage_error(error)),
        }
    }
    Err(MapBundleError::Storage(
        "bundle publication lock busy".to_owned(),
    ))
}

pub(crate) fn ensure_store_capacity(root: &Path) -> Result<(), MapBundleError> {
    let digest_count = scan_bundle_directories(root)?.len();
    if digest_count >= MAP_MAX_FEED_ENTRIES {
        return Err(MapBundleError::TooLarge("bundle store capacity"));
    }
    Ok(())
}

pub(crate) fn scan_bundle_directories(root: &Path) -> Result<Vec<String>, MapBundleError> {
    let mut digests = Vec::new();
    for (index, entry) in fs::read_dir(root).map_err(storage_error)?.enumerate() {
        if index >= MAP_MAX_STORE_SCAN_ENTRIES {
            return Err(MapBundleError::TooLarge("bundle store scan"));
        }
        let entry = entry.map_err(storage_error)?;
        let file_type = entry.file_type().map_err(storage_error)?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !is_digest(name) {
            continue;
        }
        if file_type.is_symlink() || !file_type.is_dir() {
            return Err(MapBundleError::Storage(
                "bundle digest entry is not a regular directory".to_owned(),
            ));
        }
        digests.push(name.to_owned());
        if digests.len() > MAP_MAX_FEED_ENTRIES {
            return Err(MapBundleError::TooLarge("bundle store capacity"));
        }
    }
    Ok(digests)
}

pub(crate) fn write_bundle_files(
    directory: &Path,
    bundle: &MapViewBundle,
) -> Result<(), MapBundleError> {
    feed::write_file(
        &directory.join("manifest.json"),
        &bundle.canonical_manifest_bytes()?,
    )?;
    feed::write_file(
        &directory.join(&bundle.manifest.contents.snapshot_ref),
        &bundle.snapshot_bytes,
    )?;
    feed::write_file(
        &directory.join(&bundle.manifest.contents.analysis_ref),
        &bundle.analysis_bytes()?,
    )?;
    if let (Some(reference), Some(bytes)) = (
        bundle.manifest.contents.svg_ref.as_ref(),
        bundle.svg.as_deref(),
    ) {
        feed::write_file(&directory.join(reference), bytes)?;
    }
    if let (Some(reference), Some(bytes)) = (
        bundle.manifest.contents.png_ref.as_ref(),
        bundle.png.as_deref(),
    ) {
        feed::write_file(&directory.join(reference), bytes)?;
    }
    feed::write_file(
        &directory.join(&bundle.manifest.contents.decision_ref),
        bundle
            .decision
            .as_deref()
            .ok_or(MapBundleError::DigestMismatch("decision_digest"))?,
    )?;
    feed::write_file(
        &directory.join(&bundle.manifest.contents.viewer_ref),
        bundle
            .viewer
            .as_deref()
            .ok_or(MapBundleError::DigestMismatch("viewer_digest"))?,
    )?;
    Ok(())
}

pub(crate) fn read_optional(
    directory: &Path,
    reference: Option<&str>,
) -> Result<Option<Vec<u8>>, MapBundleError> {
    reference
        .map(|name| feed::read_bounded(&directory.join(name)))
        .transpose()
}

pub(crate) fn validate_operation_id(operation_id: &str) -> Result<(), MapBundleError> {
    if operation_id.is_empty()
        || operation_id.len() > 96
        || !operation_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(MapBundleError::InvalidField("operation_id"));
    }
    Ok(())
}

pub(crate) fn reject_symlink_directory(path: &Path) -> Result<(), MapBundleError> {
    let metadata = fs::symlink_metadata(path).map_err(storage_error)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(MapBundleError::Storage(
            "bundle directory is not a regular directory".to_owned(),
        ));
    }
    Ok(())
}

pub(crate) fn directory_present(path: &Path) -> Result<bool, MapBundleError> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            reject_symlink_directory(path)?;
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(storage_error(error)),
    }
}

pub(crate) fn storage_error(error: std::io::Error) -> MapBundleError {
    MapBundleError::Storage(error.to_string())
}
