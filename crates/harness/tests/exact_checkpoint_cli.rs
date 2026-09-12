// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use sts2_harness::{ExactArtifactStore, ExactCheckpointId};

const PAYLOAD: &[u8] =
    include_bytes!("../../../protocol-artifact/exact-state-v1/golden/state.canonical");

fn workspace(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("test clock is after the unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "sts2-harness-cli-{name}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("test workspace is creatable");
    path
}

fn blob(seed: char) -> String {
    if seed == 'c' {
        return "sha256:95c56e287c1e000c70b33fc0180caf7c58f1da0606681e18a67c43bfc6ab8974"
            .to_owned();
    }
    if seed == 'd' {
        return "sha256:2fa9e73895d97193dc0153eae157f93da76c3292a526a4bbe2486e755e93b82b"
            .to_owned();
    }
    format!("sha256:{}", seed.to_string().repeat(64))
}

fn state(seed: char) -> String {
    if seed == 'a' {
        let mut hash = Sha256::new();
        hash.update(b"AI-ASCENSION/EXACT-STATE/v1\0");
        hash.update(PAYLOAD);
        let hex: String = hash
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let digest = format!("asc-state:v1:sha256:{hex}");
        return digest;
    }
    format!("asc-state:v1:sha256:{}", seed.to_string().repeat(64))
}

fn fixture() -> (PathBuf, ExactCheckpointId) {
    let root = workspace("store");
    let store = ExactArtifactStore::new(&root);
    let payload = store.stage_blob(PAYLOAD).expect("payload stages");
    let restore = store.stage_blob(b"restore").expect("restore stages");
    let manifest = serde_json::to_vec(&serde_json::json!({
        "schema": "ascension.checkpoint_manifest.v1",
        "canonical_profile": "asc-jcs-state-v1",
        "boundary": {"kind":"decision", "phase":"CONTRACT_FIXTURE"},
        "origin": {"run_id":"synthetic", "generation":0},
        "parent_checkpoint_id": null,
        "exact_state_digest": state('a'),
        "canonical_payload": {"digest": payload.as_str(), "role":"exact_state_payload", "codec":"asc-jcs-state-v1", "size_bytes":PAYLOAD.len()},
        "restore_artifacts": [{"digest": restore.as_str(), "role":"snapshot", "codec":"test-raw-v1", "size_bytes":7}],
        "compatibility_digest": blob('c'),
        "coverage_contract_digest": blob('d'),
    }))
    .expect("manifest serializes");
    let identifier = store
        .publish_manifest(&manifest)
        .expect("manifest publishes");
    (root, identifier)
}

fn run(arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_exact-checkpoint-cli"))
        .args(arguments)
        .output()
        .expect("cli runs")
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("utf8 stdout")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("utf8 stderr")
}

#[test]
fn list_and_inspect_expose_stored_identities() {
    let (root, identifier) = fixture();
    let root = root.to_str().expect("root path is utf8");
    let listed = run(&["list", "--root", root]);
    assert!(listed.status.success());
    assert!(stdout(&listed).contains(identifier.as_str()));

    let inspected = run(&[
        "inspect",
        "--root",
        root,
        "--checkpoint",
        identifier.as_str(),
    ]);
    assert!(inspected.status.success());
    let summary = stdout(&inspected);
    assert!(summary.contains(identifier.as_str()));
    assert!(summary.contains(&state('a')));
    assert!(summary.contains(&blob('c')));
}

#[test]
fn verify_reports_integrity_and_contract_failures() {
    let (root, identifier) = fixture();
    let root = root.to_str().expect("root path is utf8");
    let verified = run(&[
        "verify",
        "--root",
        root,
        "--checkpoint",
        identifier.as_str(),
        "--state-digest",
        &state('a'),
        "--compatibility",
        &blob('c'),
        "--coverage",
        &blob('d'),
    ]);
    assert!(verified.status.success());
    assert!(stdout(&verified).contains("\"status\":\"verified\""));
    assert!(stdout(&verified).contains("integrity_verified"));

    let rejected = run(&[
        "verify",
        "--root",
        root,
        "--checkpoint",
        identifier.as_str(),
        "--state-digest",
        &state('a'),
        "--compatibility",
        &blob('c'),
        "--coverage",
        &blob('e'),
    ]);
    assert_eq!(rejected.status.code(), Some(1));
    assert!(stderr(&rejected).contains("coverage_incomplete"));
}

#[test]
fn unsupported_and_unknown_commands_fail_without_approximating() {
    let (root, _identifier) = fixture();
    let root = root.to_str().expect("root path is utf8");
    let restore = run(&["restore", "--root", root]);
    assert_eq!(restore.status.code(), Some(1));
    assert!(stderr(&restore).contains("live runtime"));

    let unknown = run(&["explode", "--root", root]);
    assert_eq!(unknown.status.code(), Some(2));
    assert!(stderr(&unknown).contains("unknown command"));

    let missing = run(&["list"]);
    assert_eq!(missing.status.code(), Some(2));
    assert!(stderr(&missing).contains("usage"));
}
