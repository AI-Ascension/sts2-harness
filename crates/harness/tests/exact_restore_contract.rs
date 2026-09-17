// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const NEUTRAL_SCHEMA_DIGEST: &str =
    "2289d888c33eac46873408303c4423eab762e3f7bd6132ae8ae88d0d3b1858e4";
const GATEWAY_SCHEMA_DIGEST: &str =
    "0b181dc30524c8b14dea73e490da55538f2d57fe87bf58ed9fe33223406a7d89";
const NEUTRAL_CONTRACT: &str = "sts2-exact-restore-v1";
const GATEWAY_CONTRACT: &str = "sts2-exact-restore-gateway-v1";

fn artifact_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../contract-artifact")
        .join(name)
}

fn read(root: &Path, name: &str) -> Vec<u8> {
    std::fs::read(root.join(name)).expect("contract artifact file is present")
}

fn json_file(root: &Path, name: &str) -> Value {
    serde_json::from_slice(&read(root, name)).expect("contract artifact JSON is valid")
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn validate_inventory(root: &Path, expected_count: usize) {
    let inventory =
        String::from_utf8(read(root, "SHA256SUMS")).expect("checksum inventory is UTF-8");
    let mut count = 0;
    for line in inventory.lines() {
        let (expected, name) = line
            .split_once("  ")
            .expect("checksum inventory line has two fields");
        assert_eq!(
            sha256(&read(root, name)),
            expected,
            "checksum mismatch for {name}"
        );
        count += 1;
    }
    assert_eq!(count, expected_count);
}

fn validator(root: &Path) -> jsonschema::Validator {
    let schema = json_file(root, "schema.json");
    jsonschema::draft202012::options()
        .build(&schema)
        .expect("closed frame schema compiles")
}

fn assert_frame_size(frame: &Value) {
    let encoded = serde_json::to_vec(frame).expect("fixture frame serializes");
    assert!(
        encoded.len() <= 16_384,
        "full frame is {} bytes",
        encoded.len()
    );
}

fn assert_ids_match(wrapper: &Value) {
    let inner = &wrapper["payload"]["frame"];
    assert_eq!(wrapper["message_id"], inner["message_id"]);
    assert_eq!(wrapper["correlation_id"], inner["correlation_id"]);
}

#[test]
fn copied_protocol_and_wrapper_artifacts_match_the_frozen_pins() {
    let neutral = artifact_path("exact-restore-v1");
    let neutral_manifest = json_file(&neutral, "manifest.json");
    assert_eq!(neutral_manifest["protocol_version"], "exact-restore-v1");
    assert_eq!(neutral_manifest["schema_digest"], NEUTRAL_SCHEMA_DIGEST);
    assert_eq!(
        sha256(&read(&neutral, "schema.json")),
        NEUTRAL_SCHEMA_DIGEST
    );
    assert_eq!(
        neutral_manifest["consumers"],
        json!([
            "sts2-game-mod",
            "sts2-gateway",
            "sts2-harness",
            "sts2-mcp-server"
        ])
    );
    validate_inventory(&neutral, 6);

    let gateway = artifact_path("exact-restore-gateway-v1");
    let gateway_manifest = json_file(&gateway, "manifest.json");
    assert_eq!(gateway_manifest["contract"], GATEWAY_CONTRACT);
    assert_eq!(gateway_manifest["schema_digest"], GATEWAY_SCHEMA_DIGEST);
    assert_eq!(
        sha256(&read(&gateway, "schema.json")),
        GATEWAY_SCHEMA_DIGEST
    );
    assert_eq!(
        gateway_manifest["neutral_protocol"]["schema_digest"],
        NEUTRAL_SCHEMA_DIGEST
    );
    assert_eq!(
        gateway_manifest["neutral_protocol"]["producer_commit"],
        "5d5a368ef8a89fd1cb356b04dbf9d8a056adbf05"
    );
    assert_eq!(
        gateway_manifest["tools"],
        json!([
            "sts2.exact_restore.begin",
            "sts2.exact_restore.put_chunk",
            "sts2.exact_restore.finish_blob",
            "sts2.exact_restore.commit",
            "sts2.exact_restore.lookup"
        ])
    );
    assert_eq!(
        gateway_manifest["gateway_routes"],
        json!([
            "POST /v1/exact-restore/begin",
            "POST /v1/exact-restore/chunk",
            "POST /v1/exact-restore/finish",
            "POST /v1/exact-restore/commit",
            "POST /v1/exact-restore/lookup"
        ])
    );
    assert_eq!(gateway_manifest["limits"]["max_wrapper_bytes"], 16_384);
    assert_eq!(
        gateway_manifest["limits"]["max_neutral_frame_bytes"],
        16_384
    );
    assert_eq!(gateway_manifest["limits"]["max_chunk_raw_bytes"], 8_192);
    assert_eq!(gateway_manifest["limits"]["max_chunk_base64_bytes"], 10_924);
    validate_inventory(&gateway, 7);
}

#[test]
fn neutral_golden_frames_validate_and_carry_the_frozen_schema_digest() {
    let neutral = artifact_path("exact-restore-v1");
    let validate = validator(&neutral);
    let frames = json_file(&neutral, "golden/frames.json");
    let frames = frames["frames"].as_array().expect("golden frame array");
    assert!(!frames.is_empty());
    for frame in frames {
        assert_eq!(frame["contract"], NEUTRAL_CONTRACT);
        assert_eq!(frame["schema_digest"], NEUTRAL_SCHEMA_DIGEST);
        validate
            .validate(frame)
            .expect("neutral golden frame validates");
        assert_frame_size(frame);
    }
}

#[test]
fn wrapper_goldens_bind_identity_and_enforce_request_response_direction() {
    let neutral = artifact_path("exact-restore-v1");
    let gateway = artifact_path("exact-restore-gateway-v1");
    let neutral_validate = validator(&neutral);
    let gateway_validate = validator(&gateway);
    let request = json_file(&gateway, "golden/begin-request.json");
    let response = json_file(&gateway, "golden/begin-response.json");
    let unknown_commit = json_file(&gateway, "golden/commit-unknown-response.json");

    for wrapper in [&request, &response, &unknown_commit] {
        assert_eq!(wrapper["contract"], GATEWAY_CONTRACT);
        assert_eq!(wrapper["schema_digest"], GATEWAY_SCHEMA_DIGEST);
        assert_eq!(wrapper["auth"]["capability"], "exact_restore");
        assert_eq!(
            wrapper["actor"]["principal_id"],
            wrapper["auth"]["principal_id"]
        );
        assert_eq!(wrapper["auth"]["proof"], Value::Null);
        assert_ids_match(wrapper);
        gateway_validate
            .validate(wrapper)
            .expect("closed authenticated wrapper validates");
        neutral_validate
            .validate(&wrapper["payload"]["frame"])
            .expect("inner neutral frame validates");
        assert_frame_size(wrapper);
        assert_frame_size(&wrapper["payload"]["frame"]);
    }
    assert_eq!(request["actor"]["role"], "harness");
    assert_eq!(request["kind"], "exact_restore_request");
    assert_eq!(response["actor"]["role"], "gateway");
    assert_eq!(response["kind"], "exact_restore_response");
    assert_eq!(
        unknown_commit["payload"]["frame"]["payload"]["result"],
        "UNKNOWN"
    );

    let mut response_inside_request = request.clone();
    response_inside_request["payload"]["frame"]["kind"] = json!("exact_restore_begin_response");
    assert!(gateway_validate.validate(&response_inside_request).is_err());
    let mut request_inside_response = response.clone();
    request_inside_response["payload"]["frame"]["kind"] = json!("exact_restore_begin_request");
    assert!(gateway_validate.validate(&request_inside_response).is_err());
}

#[test]
fn committed_wrapper_rejection_vectors_are_recorded() {
    let gateway = artifact_path("exact-restore-gateway-v1");
    let rejections = json_file(&gateway, "golden/rejections.json");
    let invalid = rejections["invalid"]
        .as_array()
        .expect("wrapper rejection vectors");
    assert_eq!(invalid.len(), 2);
    assert_eq!(invalid[0]["wrapper_kind"], "exact_restore_request");
    assert_eq!(invalid[0]["inner_kind"], "exact_restore_begin_response");
    assert_eq!(invalid[1]["wrapper_kind"], "exact_restore_response");
    assert_eq!(invalid[1]["inner_kind"], "exact_restore_begin_request");
}
