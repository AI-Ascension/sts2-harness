// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use serde_json::{Value, json};
use sts2_harness::management::{
    ErrorClass, ErrorResponse, ManagementError, SubmissionRecoveryResponse,
};

#[test]
fn owner_vectors_are_closed_and_round_trip_through_rust_contracts() {
    let error_schema: Value = serde_json::from_str(include_str!(
        "../../../contracts/management-recovery/error-v2.schema.json"
    ))
    .expect("error schema");
    let submission_schema: Value = serde_json::from_str(include_str!(
        "../../../contracts/management-recovery/submission-v1.schema.json"
    ))
    .expect("submission schema");
    let vectors: Value = serde_json::from_str(include_str!(
        "../../../contracts/management-recovery/vectors.json"
    ))
    .expect("vectors");
    let error_validator = jsonschema::validator_for(&error_schema).expect("error validator");
    let recovery_validator =
        jsonschema::validator_for(&submission_schema).expect("recovery validator");
    let wire_error = &vectors["error"];
    assert!(error_validator.is_valid(wire_error));
    let decoded: ErrorResponse = serde_json::from_value(wire_error.clone()).expect("Rust error");
    assert_eq!(serde_json::to_value(decoded).expect("encode"), *wire_error);
    for recovery in vectors["recovery"].as_array().expect("recovery vectors") {
        assert!(recovery_validator.is_valid(recovery));
        let decoded: SubmissionRecoveryResponse =
            serde_json::from_value(recovery.clone()).expect("Rust recovery");
        assert_eq!(serde_json::to_value(decoded).expect("encode"), *recovery);
        let mut extra = recovery.clone();
        extra["actor_digest"] = json!("must never be exposed");
        assert!(!recovery_validator.is_valid(&extra));
    }
    let mut old_envelope = wire_error.clone();
    old_envelope["schema_version"] = json!("ascension.management/v1");
    assert!(!error_validator.is_valid(&old_envelope));
    let mut extra_error = wire_error.clone();
    extra_error["error"]["retry_automatically"] = json!(true);
    assert!(!error_validator.is_valid(&extra_error));
    let mut false_missing = vectors["recovery"][0].clone();
    false_missing["workflow_run_id"] = json!("private-run");
    assert!(!recovery_validator.is_valid(&false_missing));
    let mut false_submitted = vectors["recovery"][3].clone();
    false_submitted["response"] = Value::Null;
    assert!(!recovery_validator.is_valid(&false_submitted));
}

#[test]
fn only_write_uncertainty_uses_the_new_error_version() {
    let unknown = ManagementError::write_state_unknown("unknown", "recover original identity")
        .error_response();
    assert_eq!(unknown.schema_version, "ascension.management-error/v2");
    assert_eq!(unknown.error.class, ErrorClass::WriteStateUnknown);
    for class in [
        ErrorClass::InvalidInput,
        ErrorClass::Capability,
        ErrorClass::Conflict,
        ErrorClass::Forbidden,
        ErrorClass::Unresolved,
        ErrorClass::Unavailable,
        ErrorClass::Budget,
        ErrorClass::Store,
        ErrorClass::Replay,
        ErrorClass::Authentication,
    ] {
        let response =
            ManagementError::new(class.clone(), "existing", "existing error").error_response();
        assert_eq!(response.schema_version, "ascension.management/v1");
        assert_eq!(response.error.class, class);
    }
}
