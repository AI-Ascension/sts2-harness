// SPDX-License-Identifier: MIT

use super::bundle::{MAP_MAX_BUNDLE_BYTES, MapBundleError, MapViewBundle};
use super::bundle_validation::is_digest;
use super::canonical::{canonical_bytes, reject_duplicate_keys};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAP_FEED_VERSION: &str = "sts2.map-feed-v1";
pub const MAP_FEED_FILE: &str = "feed.json";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapFeedEntry {
    pub sequence: u64,
    pub bundle_digest: String,
    pub source_state_id: String,
    pub generation: u64,
    pub run_id: String,
    pub episode_id: String,
    pub trajectory_id: String,
    pub observed_at_unix_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapFeed {
    pub feed_version: String,
    /// The sequence assigned to the newest entry. It is zero only for an empty feed.
    pub sequence: u64,
    pub head: Option<String>,
    /// Entries are in ascending sequence order. The oldest entries may be trimmed.
    pub entries: Vec<MapFeedEntry>,
}

impl Default for MapFeed {
    fn default() -> Self {
        Self {
            feed_version: MAP_FEED_VERSION.to_owned(),
            sequence: 0,
            head: None,
            entries: Vec::new(),
        }
    }
}

impl MapFeed {
    pub(crate) fn validate(&self) -> Result<(), MapBundleError> {
        if self.feed_version != MAP_FEED_VERSION {
            return Err(MapBundleError::UnsupportedVersion(
                self.feed_version.clone(),
            ));
        }
        if self.entries.len() > super::bundle_store::MAP_MAX_FEED_ENTRIES {
            return Err(MapBundleError::TooLarge("bundle feed"));
        }
        let mut previous = None;
        let mut seen = std::collections::BTreeSet::new();
        for entry in &self.entries {
            if !is_digest(&entry.bundle_digest)
                || entry.source_state_id.is_empty()
                || entry.source_state_id.len() > 512
                || entry.run_id.is_empty()
                || entry.run_id.len() > 512
                || entry.episode_id.is_empty()
                || entry.episode_id.len() > 512
                || entry.trajectory_id.is_empty()
                || entry.trajectory_id.len() > 512
                || !seen.insert(entry.bundle_digest.clone())
            {
                return Err(MapBundleError::InvalidField("feed entry"));
            }
            if previous.is_some_and(|value| entry.sequence <= value) {
                return Err(MapBundleError::InvalidField("feed sequence"));
            }
            previous = Some(entry.sequence);
        }
        match self.entries.last() {
            None if self.sequence != 0 || self.head.is_some() => {
                Err(MapBundleError::InvalidField("empty feed head"))
            }
            None => Ok(()),
            Some(entry)
                if self.sequence != entry.sequence
                    || self.head.as_deref() != Some(entry.bundle_digest.as_str()) =>
            {
                Err(MapBundleError::InvalidField("feed head"))
            }
            Some(_) => Ok(()),
        }
    }

    #[must_use]
    pub fn head_digest(&self) -> Option<&str> {
        self.head.as_deref()
    }

    #[must_use]
    pub fn latest(&self) -> Option<&MapFeedEntry> {
        self.entries.last()
    }
}

pub(crate) fn now_unix_ms() -> Result<u64, MapBundleError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| MapBundleError::Storage("system clock is before UNIX epoch".to_owned()))
        .and_then(|duration| {
            u64::try_from(duration.as_millis())
                .map_err(|_| MapBundleError::Storage("system clock value is too large".to_owned()))
        })
}

pub(crate) fn read(root: &Path) -> Result<MapFeed, MapBundleError> {
    let path = root.join(MAP_FEED_FILE);
    if let Ok(metadata) = fs::symlink_metadata(&path)
        && metadata.file_type().is_symlink()
    {
        return Err(MapBundleError::Storage(
            "feed pointer is a symlink".to_owned(),
        ));
    }
    let bytes = match File::open(&path) {
        Ok(file) => read_bounded_file(file)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(MapFeed::default()),
        Err(error) => return Err(storage_error(error)),
    };
    reject_duplicate_keys(&bytes).map_err(MapBundleError::Canonical)?;
    let feed: MapFeed =
        serde_json::from_slice(&bytes).map_err(|_| MapBundleError::Serialization)?;
    feed.validate()?;
    Ok(feed)
}

pub(crate) fn append(
    root: &Path,
    bundle: &MapViewBundle,
    operation_id: &str,
    observed_at_unix_ms: u64,
) -> Result<(), MapBundleError> {
    let mut feed = read(root)?;
    let digest = bundle.bundle_digest();
    if let Some(existing) = feed
        .entries
        .iter()
        .find(|entry| entry.bundle_digest == digest)
    {
        validate_entry_bundle(existing, bundle)?;
        return Ok(());
    }
    let sequence = feed
        .sequence
        .checked_add(1)
        .ok_or(MapBundleError::TooLarge("feed sequence"))?;
    feed.entries.push(MapFeedEntry {
        sequence,
        bundle_digest: digest.to_owned(),
        source_state_id: bundle.manifest.history.source_state_id.clone(),
        generation: bundle.manifest.history.generation,
        run_id: bundle.manifest.run_id.clone(),
        episode_id: bundle.manifest.episode_id.clone(),
        trajectory_id: bundle.manifest.trajectory_id.clone(),
        observed_at_unix_ms,
    });
    if feed.entries.len() > super::bundle_store::MAP_MAX_FEED_ENTRIES {
        let trim = feed.entries.len() - super::bundle_store::MAP_MAX_FEED_ENTRIES;
        feed.entries.drain(..trim);
    }
    feed.head = Some(digest.to_owned());
    feed.sequence = sequence;
    feed.validate()?;
    write_atomic(root, &feed, operation_id)
}

fn validate_entry_bundle(
    entry: &MapFeedEntry,
    bundle: &MapViewBundle,
) -> Result<(), MapBundleError> {
    if entry.source_state_id != bundle.manifest.history.source_state_id
        || entry.generation != bundle.manifest.history.generation
        || entry.run_id != bundle.manifest.run_id
        || entry.episode_id != bundle.manifest.episode_id
        || entry.trajectory_id != bundle.manifest.trajectory_id
    {
        return Err(MapBundleError::IdentityMismatch);
    }
    Ok(())
}

fn write_atomic(root: &Path, feed: &MapFeed, operation_id: &str) -> Result<(), MapBundleError> {
    let bytes = canonical_bytes(feed).map_err(MapBundleError::Canonical)?;
    let temporary = root.join(format!(".{MAP_FEED_FILE}.{operation_id}.tmp"));
    if temporary.exists() {
        fs::remove_file(&temporary).map_err(storage_error)?;
    }
    write_file(&temporary, &bytes)?;
    let result = fs::rename(&temporary, root.join(MAP_FEED_FILE)).map_err(storage_error);
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub(crate) fn read_bounded(path: &Path) -> Result<Vec<u8>, MapBundleError> {
    if fs::symlink_metadata(path)
        .map_err(storage_error)?
        .file_type()
        .is_symlink()
    {
        return Err(MapBundleError::Storage("symlink file reference".to_owned()));
    }
    let file = File::open(path).map_err(storage_error)?;
    read_bounded_file(file)
}

fn read_bounded_file(mut file: File) -> Result<Vec<u8>, MapBundleError> {
    let length = file.metadata().map_err(storage_error)?.len();
    if length > MAP_MAX_BUNDLE_BYTES as u64 {
        return Err(MapBundleError::TooLarge("bundle file"));
    }
    let mut bytes = Vec::with_capacity(length as usize);
    std::io::Read::by_ref(&mut file)
        .take(MAP_MAX_BUNDLE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(storage_error)?;
    if bytes.len() > MAP_MAX_BUNDLE_BYTES {
        return Err(MapBundleError::TooLarge("bundle file"));
    }
    Ok(bytes)
}

pub(crate) fn write_file(path: &Path, bytes: &[u8]) -> Result<(), MapBundleError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(storage_error)?;
    file.write_all(bytes).map_err(storage_error)?;
    file.sync_all().map_err(storage_error)
}

fn storage_error(error: std::io::Error) -> MapBundleError {
    MapBundleError::Storage(error.to_string())
}
