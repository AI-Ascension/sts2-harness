// SPDX-License-Identifier: MIT

use super::bundle::{MapBundleError, MapViewBundle};
use super::bundle_validation::{is_digest, validate_file_reference};
use super::feed;
use super::feed::MapFeed;
use std::fs;
use std::path::{Path, PathBuf};

pub const MAP_MAX_FEED_ENTRIES: usize = 4096;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublicationReceipt {
    bundle_digest: String,
    operation_id: String,
    already_present: bool,
}

impl PublicationReceipt {
    #[must_use]
    pub fn bundle_digest(&self) -> &str {
        &self.bundle_digest
    }
    #[must_use]
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }
    #[must_use]
    pub const fn already_present(&self) -> bool {
        self.already_present
    }
}

pub trait MapBundleFeed {
    fn publish(
        &self,
        bundle: &MapViewBundle,
        operation_id: &str,
    ) -> Result<PublicationReceipt, MapBundleError>;
    fn load(&self, bundle_digest: &str) -> Result<MapViewBundle, MapBundleError>;
    fn list(&self) -> Result<Vec<String>, MapBundleError>;
}

/// Filesystem persistence is outside the pure analyzer. A complete bundle is
/// built in a temporary directory and made visible by one directory rename.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BundleFileStore {
    root: PathBuf,
}

impl BundleFileStore {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, MapBundleError> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(storage_error)?;
        let root_metadata = fs::symlink_metadata(&root).map_err(storage_error)?;
        if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
            return Err(MapBundleError::Storage(
                "bundle root is not a directory".to_owned(),
            ));
        }
        Ok(Self { root })
    }

    fn bundle_dir(&self, digest: &str) -> Result<PathBuf, MapBundleError> {
        if !is_digest(digest) {
            return Err(MapBundleError::InvalidDigest("bundle_digest"));
        }
        Ok(self.root.join(digest))
    }

    /// Reads the operational feed pointer. Feed timestamps are intentionally
    /// outside the deterministic bundle manifest and therefore do not affect
    /// its digest.
    pub fn read_feed(&self) -> Result<MapFeed, MapBundleError> {
        feed::read(&self.root)
    }

    /// Publishes a deterministic observation at an explicitly supplied wall
    /// clock value. Tests and importers can use a fixed value; normal callers
    /// should use [`MapBundleFeed::publish`].
    pub fn publish_at(
        &self,
        bundle: &MapViewBundle,
        operation_id: &str,
        observed_at_unix_ms: u64,
    ) -> Result<PublicationReceipt, MapBundleError> {
        self.publish_inner(bundle, operation_id, observed_at_unix_ms)
    }

    pub fn head(&self) -> Result<Option<MapViewBundle>, MapBundleError> {
        let feed = self.read_feed()?;
        feed.head_digest()
            .map(|digest| self.load(digest))
            .transpose()
    }

    fn publish_inner(
        &self,
        bundle: &MapViewBundle,
        operation_id: &str,
        observed_at_unix_ms: u64,
    ) -> Result<PublicationReceipt, MapBundleError> {
        bundle.validate()?;
        validate_operation_id(operation_id)?;
        let digest = bundle.bundle_digest();
        let final_dir = self.bundle_dir(digest)?;
        let already_present = if directory_present(&final_dir)? {
            let existing = self.load(digest)?;
            if existing.bundle_digest() != digest {
                return Err(MapBundleError::DigestMismatch("existing bundle"));
            }
            true
        } else {
            let temporary = self.root.join(format!(".{digest}.{operation_id}.tmp"));
            fs::create_dir(&temporary).map_err(storage_error)?;
            let result = write_bundle_files(&temporary, bundle)
                .and_then(|()| fs::rename(&temporary, &final_dir).map_err(storage_error));
            if let Err(error) = result {
                let _ = fs::remove_dir_all(&temporary);
                return Err(error);
            }
            false
        };
        // The immutable directory is visible before the feed head/index is
        // advanced. If a process dies between these operations, a retry of
        // the same digest repairs the missing feed entry.
        feed::append(&self.root, bundle, operation_id, observed_at_unix_ms)?;
        Ok(PublicationReceipt {
            bundle_digest: digest.to_owned(),
            operation_id: operation_id.to_owned(),
            already_present,
        })
    }
}

impl MapBundleFeed for BundleFileStore {
    fn publish(
        &self,
        bundle: &MapViewBundle,
        operation_id: &str,
    ) -> Result<PublicationReceipt, MapBundleError> {
        let observed_at_unix_ms = feed::now_unix_ms()?;
        self.publish_inner(bundle, operation_id, observed_at_unix_ms)
    }

    fn load(&self, bundle_digest: &str) -> Result<MapViewBundle, MapBundleError> {
        let directory = self.bundle_dir(bundle_digest)?;
        reject_symlink_directory(&directory)?;
        let manifest = feed::read_bounded(&directory.join("manifest.json"))?;
        let manifest = MapViewBundle::decode_manifest(&manifest)?;
        if manifest.bundle_digest != bundle_digest {
            return Err(MapBundleError::DigestMismatch("requested bundle"));
        }
        for reference in [
            &manifest.contents.snapshot_ref,
            &manifest.contents.analysis_ref,
            &manifest.contents.decision_ref,
            &manifest.contents.viewer_ref,
        ] {
            validate_file_reference(reference)?;
        }
        for reference in [
            manifest.contents.svg_ref.as_ref(),
            manifest.contents.png_ref.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            validate_file_reference(reference)?;
        }
        let snapshot = feed::read_bounded(&directory.join(&manifest.contents.snapshot_ref))?;
        let analysis = feed::read_bounded(&directory.join(&manifest.contents.analysis_ref))?;
        let svg = read_optional(&directory, manifest.contents.svg_ref.as_deref())?;
        let png = read_optional(&directory, manifest.contents.png_ref.as_deref())?;
        let decision = read_optional(&directory, Some(&manifest.contents.decision_ref))?;
        let viewer = read_optional(&directory, Some(&manifest.contents.viewer_ref))?;
        MapViewBundle::from_manifest_and_files(
            manifest, snapshot, &analysis, svg, png, decision, viewer,
        )
    }

    fn list(&self) -> Result<Vec<String>, MapBundleError> {
        let feed = self.read_feed()?;
        if !feed.entries.is_empty() {
            return Ok(feed
                .entries
                .into_iter()
                .map(|entry| entry.bundle_digest)
                .collect());
        }
        let mut digests = Vec::new();
        for entry in fs::read_dir(&self.root).map_err(storage_error)? {
            let entry = entry.map_err(storage_error)?;
            if entry.file_type().map_err(storage_error)?.is_dir() {
                let name = entry.file_name();
                let Some(name) = name.to_str() else { continue };
                if is_digest(name) {
                    digests.push(name.to_owned());
                }
            }
            if digests.len() > MAP_MAX_FEED_ENTRIES {
                return Err(MapBundleError::TooLarge("bundle feed"));
            }
        }
        digests.sort();
        Ok(digests)
    }
}

fn write_bundle_files(directory: &Path, bundle: &MapViewBundle) -> Result<(), MapBundleError> {
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

fn read_optional(
    directory: &Path,
    reference: Option<&str>,
) -> Result<Option<Vec<u8>>, MapBundleError> {
    reference
        .map(|name| feed::read_bounded(&directory.join(name)))
        .transpose()
}

fn validate_operation_id(operation_id: &str) -> Result<(), MapBundleError> {
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

fn reject_symlink_directory(path: &Path) -> Result<(), MapBundleError> {
    let metadata = fs::symlink_metadata(path).map_err(storage_error)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(MapBundleError::Storage(
            "bundle directory is not a regular directory".to_owned(),
        ));
    }
    Ok(())
}

fn directory_present(path: &Path) -> Result<bool, MapBundleError> {
    match fs::symlink_metadata(path) {
        Ok(_) => {
            reject_symlink_directory(path)?;
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(storage_error(error)),
    }
}

fn storage_error(error: std::io::Error) -> MapBundleError {
    MapBundleError::Storage(error.to_string())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoricalActionBinding {
    pub node_id: String,
    pub action_id: String,
    pub generation: u64,
    pub dispatchable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoricalReplay {
    pub bundle_digest: String,
    pub generation: u64,
    pub bindings: Vec<HistoricalActionBinding>,
}

impl HistoricalReplay {
    #[must_use]
    pub fn from_bundle(bundle: &MapViewBundle) -> Self {
        let bindings = bundle
            .analysis
            .candidate_routes
            .iter()
            .filter_map(|route| {
                let node_id = route.nodes.get(1).or_else(|| route.nodes.first())?;
                Some(HistoricalActionBinding {
                    node_id: node_id.clone(),
                    action_id: route.first_action_id.clone(),
                    generation: bundle.manifest.history.generation,
                    dispatchable: false,
                })
            })
            .collect();
        Self {
            bundle_digest: bundle.bundle_digest().to_owned(),
            generation: bundle.manifest.history.generation,
            bindings,
        }
    }
}
