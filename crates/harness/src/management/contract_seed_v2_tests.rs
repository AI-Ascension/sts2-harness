// SPDX-License-Identifier: MIT

use super::*;

fn request(seed: serde_json::Value) -> Result<WorkflowRunRequestV2, serde_json::Error> {
    serde_json::from_value(serde_json::json!({
        "schema_version": WORKFLOW_RUN_REQUEST_V2_SCHEMA,
        "request_id": "request-v2-1",
        "definition": {"nodes": []},
        "artifact_id": null,
        "instance_id": "instance-1",
        "profile": "synthetic.workflow.v1",
        "admission": null,
        "seed": seed,
    }))
}

fn seed_request(mode: &str, seed: Option<&str>) -> serde_json::Value {
    let mut value = serde_json::json!({
        "schema_version": WORKFLOW_SEED_REQUEST_V2_SCHEMA,
        "mode": mode,
    });
    if let Some(seed) = seed {
        value["seed"] = serde_json::Value::String(seed.to_owned());
    }
    value
}

#[test]
fn derive_once_is_closed_and_contains_no_seed_or_key_selector()
-> Result<(), Box<dyn std::error::Error>> {
    let request = request(seed_request("derive_once", None))?;
    request.validate_seed()?;

    let serialized = serde_json::to_value(request)?;
    let seed = serialized
        .get("seed")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "nested seed object")
        })?;
    assert_eq!(seed.len(), 2, "only schema and mode are admitted");
    assert!(!seed.contains_key("seed"));
    assert!(!seed.contains_key("key_version"));
    assert!(!seed.contains_key("authority_id"));
    Ok(())
}

#[test]
fn explicit_seed_requires_a_canonical_seed_and_derivation_rejects_one()
-> Result<(), Box<dyn std::error::Error>> {
    request(seed_request("explicit", Some("seed-123")))?.validate_seed()?;

    assert!(
        request(seed_request("explicit", None))?
            .validate_seed()
            .is_err()
    );
    assert!(
        request(seed_request("derive_once", Some("seed-123")))?
            .validate_seed()
            .is_err()
    );
    assert!(
        request(seed_request("explicit", Some(" seed-123")))?
            .validate_seed()
            .is_err()
    );
    Ok(())
}

#[test]
fn v2_request_and_nested_seed_reject_unknown_fields() {
    let mut root = serde_json::json!({
        "schema_version": WORKFLOW_RUN_REQUEST_V2_SCHEMA,
        "request_id": "request-v2-1",
        "definition": {"nodes": []},
        "artifact_id": null,
        "instance_id": "instance-1",
        "profile": "synthetic.workflow.v1",
        "admission": null,
        "seed": seed_request("derive_once", None),
    });
    root["caller_effective_seed"] = serde_json::Value::String("caller-choice".to_owned());
    assert!(serde_json::from_value::<WorkflowRunRequestV2>(root).is_err());

    let mut nested = seed_request("derive_once", None);
    nested["key_version"] = serde_json::Value::String("v2".to_owned());
    assert!(serde_json::from_value::<SeedRequestV2>(nested).is_err());
}

#[test]
fn v2_request_rejects_unknown_schema_and_seed_mode() -> Result<(), Box<dyn std::error::Error>> {
    let mut request_value = serde_json::json!({
        "schema_version": WORKFLOW_RUN_REQUEST_V2_SCHEMA,
        "request_id": "request-v2-1",
        "definition": {"nodes": []},
        "artifact_id": null,
        "instance_id": "instance-1",
        "profile": "synthetic.workflow.v1",
        "admission": null,
        "seed": seed_request("derive_once", None),
    });
    request_value["schema_version"] =
        serde_json::Value::String("ascension.workflow-run-request/v3".to_owned());
    assert!(
        serde_json::from_value::<WorkflowRunRequestV2>(request_value)
            .ok()
            .is_some_and(|request| request.validate_seed().is_err())
    );
    assert!(request(seed_request("future_mode", None)).is_err());
    Ok(())
}
