// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::PathBuf;

use serde_json::Value;
use sha2::{Digest, Sha256};
use sts2_harness::{
    ExactAssurance, ExactCheckpointId, ExactStateDigest, MIN_HANDLE_KEY_BYTES, OccurrenceId,
    ProjectionKey,
};

fn artifact() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../protocol-artifact/exact-checkpoint-reference-v1")
}

fn read(relative: &str) -> Vec<u8> {
    std::fs::read(artifact().join(relative)).expect("artifact file is present")
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn projected() -> Value {
    let key = ProjectionKey::new(&[7_u8; MIN_HANDLE_KEY_BYTES]).expect("key");
    let checkpoint =
        ExactCheckpointId::parse(&format!("asc-checkpoint:v1:sha256:{}", "b".repeat(64)))
            .expect("checkpoint id");
    let state =
        ExactStateDigest::parse(&format!("asc-state:v1:sha256:{}", "a".repeat(64))).expect("state");
    let occurrence = OccurrenceId::parse("run:alpha:1").expect("occurrence");
    let summary = key
        .project(
            &checkpoint,
            &state,
            &occurrence,
            "decision",
            "COMBAT",
            ExactAssurance::RestoreVerified,
        )
        .expect("summary builds");
    serde_json::to_value(&summary).expect("summary serializes")
}

#[test]
fn copied_reference_artifact_checksums_match() {
    let inventory = String::from_utf8(read("SHA256SUMS")).expect("checksums are UTF-8");
    let mut verified = 0;
    for line in inventory.lines() {
        let (expected, relative) = line.split_once("  ").expect("checksum line");
        assert_eq!(
            hex(&read(relative)),
            expected,
            "checksum mismatch for {relative}"
        );
        verified += 1;
    }
    assert_eq!(verified, 7);
}

#[test]
fn projected_summary_satisfies_the_shared_public_envelope() {
    let schema: Value = serde_json::from_slice(&read("schema.json")).expect("schema JSON");
    let validator = jsonschema::draft202012::options()
        .build(&schema)
        .expect("schema compiles");
    let summary = projected();
    validator
        .validate(&summary)
        .expect("projected summary satisfies the protocol envelope");
    assert_eq!(summary["restore_verified"], true);
    assert_eq!(summary["assurance"], "restore_verified");
}

#[test]
fn privileged_or_future_references_are_rejected_by_the_shared_schema() {
    let schema: Value = serde_json::from_slice(&read("schema.json")).expect("schema JSON");
    let validator = jsonschema::draft202012::options()
        .build(&schema)
        .expect("schema compiles");
    let mut privileged = projected();
    privileged["exact_state_digest"] =
        Value::String(format!("asc-state:v1:sha256:{}", "a".repeat(64)));
    assert!(validator.validate(&privileged).is_err());

    let mut future = projected();
    future["reference_version"] = Value::String("exact-checkpoint-reference-v2".to_owned());
    assert!(validator.validate(&future).is_err());

    let schema_json: Value = serde_json::from_slice(&read("schema.json")).expect("schema JSON");
    let properties = schema_json["properties"].as_object().expect("properties");
    assert!(properties.keys().all(|name| !name.contains("digest")));
}

#[test]
fn observation_only_assurance_cannot_be_projected_as_a_checkpoint() {
    let key = ProjectionKey::new(&[8_u8; MIN_HANDLE_KEY_BYTES]).expect("key");
    let checkpoint =
        ExactCheckpointId::parse(&format!("asc-checkpoint:v1:sha256:{}", "b".repeat(64)))
            .expect("checkpoint id");
    let state =
        ExactStateDigest::parse(&format!("asc-state:v1:sha256:{}", "a".repeat(64))).expect("state");
    let occurrence = OccurrenceId::parse("run:alpha:1").expect("occurrence");
    assert!(
        key.project(
            &checkpoint,
            &state,
            &occurrence,
            "decision",
            "COMBAT",
            ExactAssurance::PublicObservationOnly,
        )
        .is_err()
    );
}
