// SPDX-License-Identifier: MIT

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use sts2_harness::worker_handoff::{MAX_FRAME_BYTES, SCHEMA_DIGEST, WorkerCommand, WorkerRequest};

fn artifact() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../protocol-artifact/watchdog-worker-v1")
}

fn dispatch() -> String {
    std::fs::read_to_string(artifact().join("fixtures/valid/dispatch.json")).expect("fixture")
}

#[test]
fn copied_schema_is_exact_and_all_request_goldens_decode() {
    let schema = std::fs::read(artifact().join("schema.json")).expect("schema");
    assert_eq!(format!("{:x}", Sha256::digest(schema)), SCHEMA_DIGEST);
    for (name, command) in [
        ("probe-request", WorkerCommand::Probe),
        ("dispatch", WorkerCommand::Dispatch),
        ("lookup-request", WorkerCommand::Lookup),
        ("acknowledge-request", WorkerCommand::Acknowledge),
        ("control-request", WorkerCommand::SetControlMode),
    ] {
        let bytes =
            std::fs::read(artifact().join(format!("fixtures/valid/{name}.json"))).expect("fixture");
        assert_eq!(
            WorkerRequest::decode(&bytes).expect(name).command(),
            command
        );
    }
}

#[test]
fn every_invalid_artifact_fails_closed() {
    for entry in std::fs::read_dir(artifact().join("fixtures/invalid")).expect("fixtures") {
        let path = entry.expect("entry").path();
        let bytes = std::fs::read(&path).expect("fixture");
        assert!(WorkerRequest::decode(&bytes).is_err(), "{}", path.display());
    }
}

#[test]
fn signed_fractional_exponent_and_out_of_range_numbers_are_rejected() {
    let source = dispatch();
    for spelling in ["-0", "-1", "1.0", "1e0", "1E+0", "01", "9007199254740992"] {
        let bytes = source.replace(
            "\"attempt_number\": 1",
            &format!("\"attempt_number\": {spelling}"),
        );
        assert!(
            WorkerRequest::decode(bytes.as_bytes()).is_err(),
            "{spelling}"
        );
    }
    let maximum = source.replace(
        "\"attempt_number\": 1",
        "\"attempt_number\": 9007199254740991",
    );
    assert!(WorkerRequest::decode(maximum.as_bytes()).is_ok());
}

#[test]
fn each_required_field_is_required_and_unknown_fields_are_closed() {
    let original: Value = serde_json::from_str(&dispatch()).expect("fixture");
    for key in original.as_object().expect("object").keys() {
        let mut missing = original.clone();
        missing.as_object_mut().expect("object").remove(key);
        assert!(
            WorkerRequest::decode(&serde_json::to_vec(&missing).expect("JSON")).is_err(),
            "{key}"
        );
    }
    let mut extra = original;
    extra["unexpected"] = Value::Null;
    assert!(WorkerRequest::decode(&serde_json::to_vec(&extra).expect("JSON")).is_err());
}

#[test]
fn namespace_utf8_byte_and_schema_checks_are_enforced() {
    let original: Value = serde_json::from_str(&dispatch()).expect("fixture");
    for (key, value) in [
        ("run_id", original["handoff_id"].clone()),
        ("schema_digest", Value::String("0".repeat(64))),
        ("job_id", Value::String("é".repeat(65))),
        ("attempt_id", Value::String("bad\u{7f}id".into())),
        ("worker_owner_id", Value::String("../escape/owner".into())),
        ("timeout_ms", Value::from(5001)),
        ("mode_sequence", Value::from(0)),
        ("direction", Value::String("response".into())),
    ] {
        let mut invalid = original.clone();
        invalid[key] = value;
        assert!(
            WorkerRequest::decode(&serde_json::to_vec(&invalid).expect("JSON")).is_err(),
            "{key}"
        );
    }
}

#[test]
fn encoded_duplicate_keys_trailing_values_and_frame_overflow_are_rejected() {
    let source = dispatch();
    let duplicate = source.replacen('{', "{\"\\u0063ommand\":\"dispatch\",", 1);
    assert!(WorkerRequest::decode(duplicate.as_bytes()).is_err());
    assert!(WorkerRequest::decode(format!("{source} {{}}").as_bytes()).is_err());
    assert!(WorkerRequest::decode(&vec![b' '; MAX_FRAME_BYTES + 1]).is_err());
    let mut exact = source.into_bytes();
    exact.resize(MAX_FRAME_BYTES, b' ');
    assert!(WorkerRequest::decode(&exact).is_ok());
}
