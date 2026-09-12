// SPDX-License-Identifier: MIT

//! Retention, recovery, and reachability for the exact checkpoint artifact store.
//!
//! Garbage collection is reference-aware: a blob shared by two manifests survives while any
//! manifest that references it is pinned, and an absent dependency fails the sweep closed instead
//! of silently publishing a partial checkpoint. Leftover staging files from a crash are removed on
//! recovery; an unpublished manifest never becomes discoverable, and a published manifest cannot
//! expose a missing dependency because publication verifies every reference first.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::exact_checkpoint::{
    BLOB_DIGEST_PREFIX, BlobDigest, EXACT_CHECKPOINT_ID_PREFIX, ExactArtifactStore,
    ExactCheckpointError, ExactCheckpointId,
};

/// Hexadecimal length of a SHA-256 digest body.
const HEX_DIGITS: usize = 64;
/// Prefix of a staging file created before an atomic rename.
const TEMPORARY_PREFIX: &str = ".tmp-";

impl ExactArtifactStore {
    /// Returns the configured store root.
    #[must_use]
    pub fn root_directory(&self) -> &Path {
        &self.root
    }

    /// Lists stored blobs by content identity.
    pub fn stored_blobs(&self) -> Result<Vec<BlobDigest>, ExactCheckpointError> {
        let mut digests = Vec::new();
        for hex in collect_digests(&self.root.join("exact").join("blobs"))? {
            digests.push(BlobDigest::parse(&format!("{BLOB_DIGEST_PREFIX}{hex}"))?);
        }
        digests.sort();
        Ok(digests)
    }

    /// Lists published manifests by immutable identifier.
    pub fn stored_manifests(&self) -> Result<Vec<ExactCheckpointId>, ExactCheckpointError> {
        let mut identifiers = Vec::new();
        for hex in collect_digests(&self.root.join("exact").join("manifests"))? {
            identifiers.push(ExactCheckpointId::parse(&format!(
                "{EXACT_CHECKPOINT_ID_PREFIX}{hex}"
            ))?);
        }
        identifiers.sort();
        Ok(identifiers)
    }

    /// Removes staging files left by an interrupted publication and returns how many were removed.
    pub fn recover_temporaries(&self) -> Result<usize, ExactCheckpointError> {
        let mut removed = 0;
        for kind in ["blobs", "manifests"] {
            let directory = self.root.join("exact").join(kind);
            for entry in read_directory(&directory)? {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                for staged in read_directory(&path)? {
                    let staged_path = staged.path();
                    let name = staged.file_name();
                    let Some(name) = name.to_str() else { continue };
                    if name.starts_with(TEMPORARY_PREFIX) {
                        fs::remove_file(&staged_path).map_err(persistence)?;
                        removed += 1;
                    }
                }
            }
        }
        Ok(removed)
    }

    /// Removes one stored blob, reporting whether it existed.
    pub fn remove_blob(&self, digest: &BlobDigest) -> Result<bool, ExactCheckpointError> {
        let hex = digest.as_str().trim_start_matches(BLOB_DIGEST_PREFIX);
        if hex.len() != HEX_DIGITS || !valid_hex(hex) {
            return Err(ExactCheckpointError::InvalidDigest);
        }
        let path = content_path(&self.root, "blobs", hex);
        match fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(persistence(error)),
        }
    }
}

/// Reachable and collectable artifacts derived from pinned manifests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactRetentionPlan {
    /// Blobs referenced by a pinned manifest.
    pub retained_blobs: Vec<BlobDigest>,
    /// Stored blobs referenced by no pinned manifest.
    pub collectable_blobs: Vec<BlobDigest>,
    /// Dependencies a pinned manifest requires but the store does not hold.
    pub missing_blobs: Vec<BlobDigest>,
}

/// Computes a retention plan from pinned checkpoint identifiers.
pub fn plan_retention(
    store: &ExactArtifactStore,
    pinned: &[ExactCheckpointId],
) -> Result<ExactRetentionPlan, ExactCheckpointError> {
    let mut retained = BTreeSet::new();
    let mut missing = BTreeSet::new();
    for identifier in pinned {
        let manifest: serde_json::Value = serde_json::from_slice(&store.read_manifest(identifier)?)
            .map_err(|_| ExactCheckpointError::InvalidManifest)?;
        for digest in referenced_blobs(&manifest)? {
            let stored = store.read_blob(&digest);
            match stored {
                Ok(_) => {
                    retained.insert(digest);
                }
                Err(ExactCheckpointError::Missing) => {
                    missing.insert(digest);
                }
                Err(error) => return Err(error),
            }
        }
    }
    let mut collectable = Vec::new();
    for digest in store.stored_blobs()? {
        if !retained.contains(&digest) {
            collectable.push(digest);
        }
    }
    Ok(ExactRetentionPlan {
        retained_blobs: retained.into_iter().collect(),
        collectable_blobs: collectable,
        missing_blobs: missing.into_iter().collect(),
    })
}

/// Deletes collectable blobs, refusing to run while a pinned dependency is missing.
pub fn sweep(
    store: &ExactArtifactStore,
    plan: &ExactRetentionPlan,
) -> Result<usize, ExactCheckpointError> {
    if !plan.missing_blobs.is_empty() {
        return Err(ExactCheckpointError::Missing);
    }
    let mut removed = 0;
    for digest in &plan.collectable_blobs {
        if store.remove_blob(digest)? {
            removed += 1;
        }
    }
    Ok(removed)
}

/// Collects blob digests referenced by a manifest's payload and restore artifacts.
fn referenced_blobs(manifest: &serde_json::Value) -> Result<Vec<BlobDigest>, ExactCheckpointError> {
    let mut digests = Vec::new();
    if let Some(digest) = manifest
        .get("canonical_payload")
        .and_then(|payload| payload.get("digest"))
        .and_then(serde_json::Value::as_str)
    {
        digests.push(BlobDigest::parse(digest)?);
    }
    if let Some(entries) = manifest
        .get("restore_artifacts")
        .and_then(serde_json::Value::as_array)
    {
        for entry in entries {
            if let Some(digest) = entry.get("digest").and_then(serde_json::Value::as_str) {
                digests.push(BlobDigest::parse(digest)?);
            }
        }
    }
    Ok(digests)
}

fn collect_digests(directory: &Path) -> Result<Vec<String>, ExactCheckpointError> {
    let mut digests = Vec::new();
    for prefix in read_directory(directory)? {
        let path = prefix.path();
        if !path.is_dir() {
            continue;
        }
        for entry in read_directory(&path)? {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.len() == HEX_DIGITS && valid_hex(name) {
                digests.push(name.to_owned());
            }
        }
    }
    Ok(digests)
}

fn read_directory(directory: &Path) -> Result<Vec<fs::DirEntry>, ExactCheckpointError> {
    match fs::read_dir(directory) {
        Ok(entries) => entries
            .collect::<Result<Vec<_>, io::Error>>()
            .map_err(persistence),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(persistence(error)),
    }
}

fn content_path(root: &Path, kind: &str, hex: &str) -> PathBuf {
    let (prefix, _) = hex.split_at(2);
    root.join("exact").join(kind).join(prefix).join(hex)
}

fn valid_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn persistence(error: io::Error) -> ExactCheckpointError {
    ExactCheckpointError::Persistence(error.to_string())
}
