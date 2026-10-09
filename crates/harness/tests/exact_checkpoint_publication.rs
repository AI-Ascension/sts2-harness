// SPDX-License-Identifier: MIT

#![cfg(target_os = "linux")]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};

use sts2_harness::{BlobDigest, ExactArtifactStore, ExactCheckpointError, plan_retention, sweep};

type TestResult = Result<(), Box<dyn std::error::Error>>;
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Workspace(PathBuf);

impl Workspace {
    fn new() -> Result<Self, std::io::Error> {
        Self::named("general")
    }

    fn named(name: &str) -> Result<Self, std::io::Error> {
        let path = std::env::temp_dir().join(format!(
            "exact-publication-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn blob_path(root: &Path, digest: &BlobDigest) -> PathBuf {
    let hex = digest.as_str().trim_start_matches("sha256:");
    root.join("exact/blobs").join(&hex[..2]).join(hex)
}

fn manifest(payload: &BlobDigest) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&serde_json::json!({
        "schema": "ascension.checkpoint_manifest.v1",
        "canonical_payload": {"digest": payload.as_str()},
        "restore_artifacts": [],
    }))
}

#[test]
fn unprivileged_publication_reopens_deduplicates_and_retains_referenced_bytes() -> TestResult {
    let workspace = Workspace::new()?;
    let root = workspace.0.join("store");
    let store = ExactArtifactStore::new(&root);
    let absent = BlobDigest::parse(&format!("sha256:{}", "e".repeat(64)))?;
    assert_eq!(store.read_blob(&absent), Err(ExactCheckpointError::Missing));

    let payload = store.stage_blob(b"exact payload")?;
    let unreferenced = store.stage_blob(b"unreferenced")?;
    let manifest_bytes = manifest(&payload)?;
    let identifier = store.publish_manifest(&manifest_bytes)?;
    let hex = identifier
        .as_str()
        .trim_start_matches("asc-checkpoint:v1:sha256:");
    let path = root.join("exact/manifests").join(&hex[..2]).join(hex);
    let entries = fs::read_dir(path.parent().ok_or("manifest shard missing")?)?
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(
        entries.len(),
        1,
        "manifest publication leaves no named staging files"
    );
    assert_eq!(store.read_manifest(&identifier)?, manifest_bytes);
    drop(store);

    let reopened = ExactArtifactStore::new(&root);
    assert_eq!(reopened.read_blob(&payload)?, b"exact payload");
    assert_eq!(reopened.stage_blob(b"exact payload")?, payload);
    assert_eq!(reopened.read_manifest(&identifier)?, manifest_bytes);

    let plan = plan_retention(&reopened, &[identifier])?;
    assert_eq!(plan.retained_blobs, vec![payload.clone()]);
    assert_eq!(plan.collectable_blobs, vec![unreferenced.clone()]);
    assert!(plan.missing_blobs.is_empty());
    assert_eq!(sweep(&reopened, &plan)?, 1);
    assert_eq!(reopened.read_blob(&payload)?, b"exact payload");
    assert_eq!(
        reopened.read_blob(&unreferenced),
        Err(ExactCheckpointError::Missing)
    );
    Ok(())
}

#[test]
fn concurrent_identical_publication_is_durable_and_deduplicated() -> TestResult {
    let workspace = Workspace::new()?;
    let store = ExactArtifactStore::new(workspace.0.join("store"));
    let digests = std::thread::scope(|scope| {
        let barrier = Arc::new(Barrier::new(8));
        let handles = (0..8)
            .map(|_| {
                let store = store.clone();
                let barrier = Arc::clone(&barrier);
                scope.spawn(move || {
                    barrier.wait();
                    store.stage_blob(b"parallel exact payload")
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .map_err(|_| std::io::Error::other("publisher thread panicked"))
            })
            .collect::<Result<Vec<_>, _>>()
    })?;
    let digests = digests.into_iter().collect::<Result<Vec<_>, _>>()?;

    assert!(digests.iter().all(|digest| digest == &digests[0]));
    assert_eq!(store.stored_blobs()?.len(), 1);
    assert_eq!(store.read_blob(&digests[0])?, b"parallel exact payload");
    Ok(())
}

#[test]
fn conflicting_bytes_at_requested_digest_are_refused_without_overwrite() -> TestResult {
    let workspace = Workspace::named("conflicting-bytes")?;
    let store = ExactArtifactStore::new(workspace.0.join("store"));
    let expected = b"project-owned expected checkpoint payload";
    let conflicting = b"different pre-existing bytes";
    let digest = store.stage_blob(expected)?;
    let leaf = blob_path(store.root_directory(), &digest);

    fs::write(&leaf, conflicting)?;
    assert_eq!(
        store.stage_blob(expected),
        Err(ExactCheckpointError::DigestMismatch)
    );
    assert_eq!(fs::read(&leaf)?, conflicting);
    Ok(())
}

#[test]
fn conflicting_hard_linked_leaf_is_refused_without_changing_sentinel() -> TestResult {
    let workspace = Workspace::named("hard-link-conflict")?;
    let store = ExactArtifactStore::new(workspace.0.join("store"));
    let expected = b"project-owned expected payload for hard-link check";
    let sentinel_bytes = b"outside-store sentinel remains unchanged";
    let digest = store.stage_blob(expected)?;
    let leaf = blob_path(store.root_directory(), &digest);
    let sentinel = workspace.0.join("sentinel");

    fs::write(&sentinel, sentinel_bytes)?;
    fs::remove_file(&leaf)?;
    fs::hard_link(&sentinel, &leaf)?;
    assert_eq!(
        store.stage_blob(expected),
        Err(ExactCheckpointError::DigestMismatch)
    );
    assert_eq!(fs::read(&leaf)?, sentinel_bytes);
    assert_eq!(fs::read(&sentinel)?, sentinel_bytes);
    Ok(())
}
