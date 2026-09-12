// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::PathBuf;

use serde_json::Value;
use sha2::{Digest, Sha256};
use sts2_harness::{
    BLOB_DIGEST_PREFIX, BlobDigest, EXACT_CHECKPOINT_ID_PREFIX, EXACT_STATE_DIGEST_PREFIX,
    ExactCheckpointId, ExactStateDigest,
};

fn artifact() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../protocol-artifact/exact-state-v1")
}

fn read(relative: &str) -> Vec<u8> {
    std::fs::read(artifact().join(relative)).expect("artifact file is present")
}

fn json(relative: &str) -> Value {
    serde_json::from_slice(&read(relative)).expect("artifact JSON is valid")
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn copied_artifact_checksums_match_its_bytes() {
    let inventory =
        String::from_utf8(read("SHA256SUMS")).expect("checksum inventory is UTF-8 text");
    let mut verified = 0;
    for line in inventory.lines() {
        let (expected, relative) = line.split_once("  ").expect("checksum line has two fields");
        assert_eq!(
            hex(&read(relative)),
            expected,
            "checksum mismatch for {relative}"
        );
        verified += 1;
    }
    assert_eq!(verified, 11);
}

#[test]
fn artifact_manifest_binds_the_schema_bytes() {
    let manifest = json("manifest.json");
    assert_eq!(manifest["canonical_profile"], "asc-jcs-state-v1");
    assert_eq!(
        manifest["schema_digest"].as_str().expect("schema digest"),
        hex(&read("schema.json"))
    );
    assert_eq!(
        manifest["consumers"].as_array().expect("consumers").len(),
        3
    );
}

#[test]
fn receipt_identifiers_use_the_local_namespaces() {
    let receipt = json("golden/receipt.json");
    let state = receipt["exact_state_digest"]
        .as_str()
        .expect("state digest");
    let checkpoint = receipt["exact_checkpoint_id"]
        .as_str()
        .expect("checkpoint id");
    let blob = receipt["canonical_payload_digest"]
        .as_str()
        .expect("blob digest");

    assert!(state.starts_with(EXACT_STATE_DIGEST_PREFIX));
    assert!(checkpoint.starts_with(EXACT_CHECKPOINT_ID_PREFIX));
    assert!(blob.starts_with(BLOB_DIGEST_PREFIX));
    assert!(ExactStateDigest::parse(state).is_ok());
    assert!(ExactCheckpointId::parse(checkpoint).is_ok());
    assert!(BlobDigest::parse(blob).is_ok());

    assert_ne!(state, checkpoint);
    assert!(ExactCheckpointId::parse(state).is_err());
    assert!(ExactStateDigest::parse(checkpoint).is_err());
}

#[test]
fn golden_manifest_validates_against_the_copied_envelope() {
    let schema: Value = serde_json::from_slice(&read("checkpoint-manifest.schema.json"))
        .expect("manifest schema is valid JSON");
    let validator = jsonschema::draft202012::options()
        .build(&schema)
        .expect("manifest schema compiles");
    let instance = json("golden/manifest.json");
    validator
        .validate(&instance)
        .expect("golden manifest validates");

    let golden = json("golden/state-payload.json");
    let state_schema: Value =
        serde_json::from_slice(&read("schema.json")).expect("state schema is valid JSON");
    jsonschema::draft202012::options()
        .build(&state_schema)
        .expect("state schema compiles")
        .validate(&golden)
        .expect("golden payload validates");

    assert_eq!(
        instance["exact_state_digest"],
        json("golden/receipt.json")["exact_state_digest"]
    );
    let canonical = read("golden/state.canonical");
    assert_eq!(canonical.first(), Some(&b'{'));
    let digest = format!("{BLOB_DIGEST_PREFIX}{}", hex(&canonical));
    assert_eq!(
        digest,
        json("golden/receipt.json")["canonical_payload_digest"]
    );
}
