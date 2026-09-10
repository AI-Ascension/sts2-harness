// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::PathBuf;
use sts2_harness::worker_handoff::{MAX_FRAME_BYTES, PAYLOAD_DIGEST, SCHEMA_DIGEST};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../protocol-artifact/watchdog-worker-v1")
}

#[test]
fn full_copied_json_inventory_is_frozen_and_complete() -> TestResult {
    let inventory = std::fs::read(root().join("SHA256SUMS"))?;
    assert_eq!(
        format!("{:x}", Sha256::digest(&inventory)),
        "7835a5396f3f9671b5dfa7e9a4f1ef4249809253a8554b6e6ad1cf6b32b502db"
    );
    let mut paths = BTreeSet::new();
    for line in std::str::from_utf8(&inventory)?.lines() {
        let (digest, path) = line.split_once("  ").ok_or("inventory row")?;
        assert!(paths.insert(path.to_owned()));
        let bytes = std::fs::read(root().join(path))?;
        assert_eq!(format!("{:x}", Sha256::digest(bytes)), digest, "{path}");
    }
    let mut actual = BTreeSet::from(["schema.json".to_owned(), "manifest.json".to_owned()]);
    for folder in ["valid", "invalid"] {
        for file in std::fs::read_dir(root().join("fixtures").join(folder))? {
            let name = file?
                .file_name()
                .into_string()
                .map_err(|_| "fixture filename")?;
            actual.insert(format!("fixtures/{folder}/{name}"));
        }
    }
    assert_eq!(paths, actual);
    assert_eq!(paths.len(), 20);
    Ok(())
}

#[test]
fn manifest_pins_the_decoder_limits_and_empty_operation_contract() -> TestResult {
    let bytes = std::fs::read(root().join("manifest.json"))?;
    assert_eq!(
        format!("{:x}", Sha256::digest(&bytes)),
        "63a66dc883da24f2be324ce6c25cf8a5e103109f15bdddd49a713d19004fe76a"
    );
    let manifest: Value = serde_json::from_slice(&bytes)?;
    assert_eq!(manifest["schema_file"], "schema.json");
    assert_eq!(manifest["schema_digest"], SCHEMA_DIGEST);
    assert_eq!(
        manifest["operation"],
        json!({
            "name":"runtime_v3_episode", "parameters":{}, "payload_digest":PAYLOAD_DIGEST
        })
    );
    assert_eq!(
        manifest["limits"],
        json!({
            "max_frame_bytes": MAX_FRAME_BYTES, "max_terminal_bytes":16384,
            "max_json_depth":16, "max_deadline_ms":5000, "max_wire_integer":9007199254740991_u64,
            "max_identity_bytes":128, "max_reference_bytes":1024
        })
    );
    let schema: Value = serde_json::from_slice(&std::fs::read(root().join("schema.json"))?)?;
    let validator = jsonschema::validator_for(&schema)?;
    for entry in std::fs::read_dir(root().join("fixtures/valid"))? {
        let path = entry?.path();
        let value: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
        assert!(validator.is_valid(&value), "{}", path.display());
    }
    Ok(())
}
