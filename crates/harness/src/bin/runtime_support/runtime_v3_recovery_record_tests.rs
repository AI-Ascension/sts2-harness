// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use sts2_harness::{
    DispatchStatus, ExecutionLineage, OperationIntent, OperationState, StoredOperation,
};

use super::{authoritative_reconcile_state, response_state, wire};

const OPERATION_ID: &str = "11111111-1111-4111-8111-111111111111";
const STATE_ID: &str = "22222222-2222-4222-8222-222222222222";
const PAYLOAD_DIGEST: &str = "150dbfb8ac3331371ecd80224e78274a8aa1d221916d867b6125bfe07ac88107";
const CATALOG_DIGEST: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const FENCE_ID: &str = "88888888-8888-4888-8888-888888888888";

fn operation() -> Result<StoredOperation, Box<dyn std::error::Error>> {
    let lineage = ExecutionLineage::new(
        "run-record",
        "episode-record",
        "attempt-record",
        "trajectory-record",
    )?;
    let intent = OperationIntent::new_with_action(
        lineage,
        OPERATION_ID,
        STATE_ID,
        0,
        "combat.end-turn",
        "end_turn",
        br#"{"action":{"kind":"end_turn"},"action_id":"combat.end-turn"}"#.to_vec(),
        PAYLOAD_DIGEST,
        PAYLOAD_DIGEST,
        Some(String::from(CATALOG_DIGEST)),
    )?;
    Ok(StoredOperation {
        intent,
        state: OperationState::Unknown,
        result_ref: None,
        result_digest: None,
    })
}

fn context() -> Value {
    json!({
        "deployment_id": "33333333-3333-4333-8333-333333333333",
        "instance_id": "44444444-4444-4444-8444-444444444444",
        "instance_incarnation": "55555555-5555-4555-8555-555555555555",
        "boot_id": "66666666-6666-4666-8666-666666666666",
        "authority_generation": 1,
        "lease_id": "77777777-7777-4777-8777-777777777777",
        "lease_epoch": 1
    })
}

fn payload(state: &str, ticket_state: &str) -> Value {
    let original = context();
    let witness = json!({
        "witness_id": "99999999-9999-4999-8999-999999999999",
        "operation_id": OPERATION_ID,
        "payload_digest": PAYLOAD_DIGEST,
        "boot_id": original["boot_id"],
        "instance_incarnation": original["instance_incarnation"],
        "host_fence_id": FENCE_ID,
        "source": "host_receipt",
        "state_id": STATE_ID,
        "generation": 1,
        "effect_digest": CATALOG_DIGEST,
        "observed_at": "2026-09-07T00:00:01Z"
    });
    json!({
        "result": {"status": state, "retryable": false, "retry_after_seconds": null},
        "witness": witness,
        "operation": {
            "operation_id": OPERATION_ID,
            "state": state,
            "payload_digest": PAYLOAD_DIGEST,
            "original_context": original,
            "expected_boundary": {"state_id": STATE_ID, "generation": 0, "catalog_digest": CATALOG_DIGEST},
            "action": {
                "schema_digest": wire::RUNTIME_V3_SCHEMA_DIGEST,
                "canonical_json_b64": "eyJhY3Rpb24iOnsia2luZCI6ImVuZF90dXJuIn0sImFjdGlvbl9pZCI6ImNvbWJhdC5lbmQtdHVybiJ9",
                "payload_digest": PAYLOAD_DIGEST
            },
            "ticket": {
                "operation_id": OPERATION_ID,
                "payload_digest": PAYLOAD_DIGEST,
                "state": ticket_state,
                "boot_id": original["boot_id"],
                "instance_incarnation": original["instance_incarnation"],
                "lease_epoch": original["lease_epoch"],
                "host_fence_id": FENCE_ID
            },
            "witness": witness
        }
    })
}

#[test]
fn authoritative_ticket_state_drives_recovery_outcome() -> Result<(), Box<dyn std::error::Error>> {
    let operation = operation()?;
    for state in ["SETTLED", "RECONCILED"] {
        assert_eq!(
            authoritative_reconcile_state(&payload(state, "SETTLED"), &operation, &context())?,
            (OperationState::Settled, DispatchStatus::Settled)
        );
    }
    for state in ["REJECTED", "RECONCILED"] {
        assert_eq!(
            authoritative_reconcile_state(&payload(state, "REJECTED"), &operation, &context())?,
            (OperationState::Rejected, DispatchStatus::Rejected)
        );
    }
    Ok(())
}

#[test]
fn settlement_without_an_operation_bound_witness_is_not_evidence()
-> Result<(), Box<dyn std::error::Error>> {
    let operation = operation()?;
    for state in ["SETTLED", "RECONCILED"] {
        let mut response = payload(state, "SETTLED");
        response["operation"]["witness"] = Value::Null;
        response["witness"] = Value::Null;
        assert!(authoritative_reconcile_state(&response, &operation, &context()).is_err());
    }
    let mut rejected = payload("REJECTED", "REJECTED");
    rejected["operation"]["witness"] = Value::Null;
    rejected["witness"] = Value::Null;
    assert_eq!(
        authoritative_reconcile_state(&rejected, &operation, &context())?.0,
        OperationState::Rejected
    );
    Ok(())
}

#[test]
fn cross_operation_authority_ticket_and_witness_substitution_is_rejected()
-> Result<(), Box<dyn std::error::Error>> {
    let operation = operation()?;
    for (field, value) in [
        ("operation_id", json!(STATE_ID)),
        ("payload_digest", json!(CATALOG_DIGEST)),
        ("boot_id", json!(STATE_ID)),
        ("instance_incarnation", json!(STATE_ID)),
        ("host_fence_id", json!(STATE_ID)),
        ("generation", json!(0)),
        ("source", json!("socket_success")),
    ] {
        let mut response = payload("RECONCILED", "SETTLED");
        response["operation"]["witness"][field] = value.clone();
        response["witness"][field] = value;
        assert!(
            authoritative_reconcile_state(&response, &operation, &context()).is_err(),
            "witness {field}"
        );
    }
    for (field, value) in [
        ("operation_id", json!(STATE_ID)),
        ("payload_digest", json!(CATALOG_DIGEST)),
        ("boot_id", json!(STATE_ID)),
        ("instance_incarnation", json!(STATE_ID)),
        ("host_fence_id", json!(STATE_ID)),
        ("lease_epoch", json!(2)),
    ] {
        let mut response = payload("RECONCILED", "SETTLED");
        response["operation"]["ticket"][field] = value;
        assert!(
            authoritative_reconcile_state(&response, &operation, &context()).is_err(),
            "ticket {field}"
        );
    }
    Ok(())
}

#[test]
fn original_context_and_duplicate_witnesses_must_match() -> Result<(), Box<dyn std::error::Error>> {
    let operation = operation()?;
    for field in [
        "deployment_id",
        "instance_id",
        "instance_incarnation",
        "boot_id",
        "authority_generation",
        "lease_id",
        "lease_epoch",
    ] {
        let mut response = payload("RECONCILED", "SETTLED");
        response["operation"]["original_context"][field] = Value::Null;
        assert!(
            authoritative_reconcile_state(&response, &operation, &context()).is_err(),
            "{field}"
        );
    }
    let mut response = payload("RECONCILED", "SETTLED");
    response["witness"]["witness_id"] = json!(STATE_ID);
    assert!(authoritative_reconcile_state(&response, &operation, &context()).is_err());
    Ok(())
}

#[test]
fn not_found_or_unresolved_states_never_manufacture_terminal_evidence()
-> Result<(), Box<dyn std::error::Error>> {
    let operation = operation()?;
    let absent = json!({"result":{"status":"NOT_FOUND"}, "operation":null});
    assert_eq!(response_state(&absent, &operation, &context())?, None);
    assert!(authoritative_reconcile_state(&absent, &operation, &context()).is_err());
    for state in [
        "INTENT_RECORDED",
        "MAY_HAVE_BEEN_DISPATCHED",
        "ACCEPTED",
        "UNKNOWN",
    ] {
        let response = payload(state, "SETTLED");
        assert_eq!(
            response_state(&response, &operation, &context())?.as_deref(),
            Some(state)
        );
        assert!(authoritative_reconcile_state(&response, &operation, &context()).is_err());
    }
    let mut response = payload("SETTLED", "REJECTED");
    assert!(authoritative_reconcile_state(&response, &operation, &context()).is_err());
    response["result"]["status"] = json!("RECONCILED");
    assert!(response_state(&response, &operation, &context()).is_err());
    Ok(())
}
