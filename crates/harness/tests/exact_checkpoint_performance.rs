// SPDX-License-Identifier: MIT

//! Declared-budget checks for the durable exact artifact store and retention.
//!
//! Ceilings are generous and cover the synthetic profile only; they catch algorithmic regressions,
//! not machine noise, and say nothing about capture pause or a real game snapshot.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sts2_harness::{ExactArtifactStore, ExactCheckpointId, plan_retention, sweep};

const MANIFESTS: usize = 50;

fn workspace(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("test clock is after the unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "sts2-harness-perf-{name}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("test workspace is creatable");
    path
}

fn state(index: usize) -> String {
    format!("asc-state:v1:sha256:{:064x}", index + 1)
}

#[test]
fn store_and_retention_stay_within_the_declared_budget() {
    let store = ExactArtifactStore::new(workspace("store"));
    let large = vec![7_u8; 1024 * 1024];
    let started = Instant::now();
    let payload = store.stage_blob(&large).expect("large blob stages");
    let _ = store.read_blob(&payload).expect("large blob reads back");
    let storage = started.elapsed();
    println!("stage+read 1 MiB blob: {storage:?}");
    assert!(
        storage < Duration::from_secs(3),
        "storage exceeded the declared budget: {storage:?}"
    );

    let started = Instant::now();
    let mut identifiers = Vec::new();
    for index in 0..MANIFESTS {
        let restore = store
            .stage_blob(format!("restore-{index}").as_bytes())
            .expect("restore blob stages");
        let manifest = serde_json::to_vec(&serde_json::json!({
            "schema": "ascension.checkpoint_manifest.v1",
            "exact_state_digest": state(index),
            "canonical_payload": {"digest": payload.as_str()},
            "restore_artifacts": [{"digest": restore.as_str()}],
            "compatibility_digest": format!("sha256:{}", "c".repeat(64)),
            "coverage_contract_digest": format!("sha256:{}", "d".repeat(64)),
        }))
        .expect("manifest serializes");
        identifiers.push(
            store
                .publish_manifest(&manifest)
                .expect("manifest publishes"),
        );
    }
    let published = store.stored_manifests().expect("manifests list");
    assert_eq!(published.len(), MANIFESTS);
    let plan = plan_retention(&store, std::slice::from_ref(&identifiers[0])).expect("plan builds");
    let removed = sweep(&store, &plan).expect("sweep succeeds");
    let maintenance = started.elapsed();
    println!(
        "publish+list {MANIFESTS} manifests and sweep retention: {maintenance:?} (removed {removed})"
    );
    assert_eq!(removed, MANIFESTS - 1);
    assert!(maintenance < Duration::from_secs(5));
    assert!(store.read_blob(&payload).is_ok());
    let first: &ExactCheckpointId = &identifiers[0];
    assert!(store.read_manifest(first).is_ok());
}

#[test]
fn stored_blob_enumeration_is_bounded_by_content_addressing() {
    let store = ExactArtifactStore::new(workspace("enumerate"));
    for index in 0..16 {
        store
            .stage_blob(format!("value-{index}").as_bytes())
            .expect("blob stages");
    }
    let started = Instant::now();
    let blobs = store.stored_blobs().expect("blobs list");
    let elapsed = started.elapsed();
    assert_eq!(blobs.len(), 16);
    assert!(elapsed < Duration::from_secs(1), "enumeration: {elapsed:?}");
}
