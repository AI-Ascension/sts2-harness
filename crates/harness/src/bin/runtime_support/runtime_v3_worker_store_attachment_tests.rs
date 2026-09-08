// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::panic)]

use sts2_harness::{Checkpoint, CompletionRecord, CompletionStatus, ExecutionFingerprint};

use super::super::durable::DurableHandle;
use super::super::worker_store::{share_store, try_lock};
use super::{
    lineage, prepared_runtime, remove_executable, runtime_config, setup_store_with_fingerprint,
};

#[test]
fn admitted_attachment_rejects_a_relative_mcp_path_without_path_lookup() {
    let config = runtime_config("sts2-mcp-server");
    assert!(super::super::durable::validate_worker_config_path_for_test(&config).is_err());
}

#[test]
fn admitted_attachment_rejects_a_relative_path_before_touching_the_store() {
    let (_path, mut config, settings, approved, shared, handoff) = prepared_runtime();
    config.mcp_binary = String::from("mcp-relative");
    let result = DurableHandle::from_admitted_shared_store(
        shared,
        &handoff,
        &config,
        &settings,
        lineage(),
        approved,
    );
    assert!(result.is_err());
}

#[test]
fn admitted_attachment_rejects_a_wrong_approved_fingerprint() {
    let (path, config, settings, _approved, shared, handoff) = prepared_runtime();
    let wrong = ExecutionFingerprint::new(
        "seed-1",
        "build-1",
        "state-1",
        "f".repeat(64),
        "a".repeat(64),
    )
    .expect("wrong fingerprint is structurally valid");
    let result = DurableHandle::from_admitted_shared_store(
        shared,
        &handoff,
        &config,
        &settings,
        lineage(),
        wrong,
    );
    remove_executable(&path);
    assert!(result.is_err());
}

#[test]
fn admitted_attachment_rejects_a_missing_handoff() {
    let (path, config, settings, approved, _running_store, handoff) = prepared_runtime();
    let (store, _tuple, _context) = setup_store_with_fingerprint(&approved);
    let result = DurableHandle::from_admitted_shared_store(
        share_store(store),
        &handoff,
        &config,
        &settings,
        lineage(),
        approved,
    );
    remove_executable(&path);
    assert!(result.is_err());
}

#[test]
fn admitted_attachment_rejects_a_completed_episode() {
    let (path, config, settings, approved, shared, handoff) = prepared_runtime();
    {
        let mut store = try_lock(&shared).expect("completion obtains store lease");
        store
            .save_checkpoint(
                &Checkpoint::new(
                    lineage(),
                    0,
                    "state-1",
                    1,
                    approved.clone(),
                    b"{}".to_vec(),
                    "catalog-1",
                )
                .expect("checkpoint"),
            )
            .expect("checkpoint saves");
        store
            .record_completion(
                &CompletionRecord::new(
                    lineage(),
                    CompletionStatus::Completed,
                    "terminal-1",
                    0,
                    "b".repeat(64),
                )
                .expect("completion"),
            )
            .expect("completion records");
    }
    let result = DurableHandle::from_admitted_shared_store(
        shared,
        &handoff,
        &config,
        &settings,
        lineage(),
        approved,
    );
    remove_executable(&path);
    assert!(result.is_err());
}
