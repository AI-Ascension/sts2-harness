// SPDX-License-Identifier: MIT

use serde_json::Value;
use sha2::Digest;
use sts2_harness::{DispatchStatus, OperationState};

use super::super::wire;

pub(super) fn operation_record(
    payload: &Value,
    operation: &sts2_harness::StoredOperation,
) -> Result<String, String> {
    let value = payload
        .get("operation")
        .filter(|value| !value.is_null())
        .ok_or_else(|| String::from("recovery response omitted the operation record"))?;
    let object = value
        .as_object()
        .ok_or_else(|| String::from("recovery operation record is not an object"))?;
    if object.get("operation_id").and_then(Value::as_str)
        != Some(operation.intent.operation_id.as_str())
        || object.get("payload_digest").and_then(Value::as_str)
            != Some(operation.intent.payload_digest.as_str())
    {
        return Err(String::from(
            "recovery operation record does not match the original payload digest",
        ));
    }
    let boundary = object
        .get("expected_boundary")
        .and_then(Value::as_object)
        .ok_or_else(|| String::from("recovery operation record omitted expected boundary"))?;
    if boundary.get("state_id").and_then(Value::as_str) != Some(operation.intent.state_id.as_str())
        || boundary.get("generation").and_then(Value::as_u64) != Some(operation.intent.generation)
        || boundary.get("catalog_digest").and_then(Value::as_str)
            != operation.intent.catalog_digest.as_deref()
    {
        return Err(String::from(
            "recovery operation record does not match the original boundary",
        ));
    }
    let action = object
        .get("action")
        .and_then(Value::as_object)
        .ok_or_else(|| String::from("recovery operation record omitted action identity"))?;
    if action.get("schema_digest").and_then(Value::as_str) != Some(wire::RUNTIME_V3_SCHEMA_DIGEST)
        || action.get("payload_digest").and_then(Value::as_str)
            != Some(operation.intent.payload_digest.as_str())
    {
        return Err(String::from(
            "recovery operation record contains an invalid canonical action identity",
        ));
    }
    let encoded = action
        .get("canonical_json_b64")
        .and_then(Value::as_str)
        .ok_or_else(|| String::from("recovery operation record omitted canonical action bytes"))?;
    let decoded = super::base64::decode(encoded).ok_or_else(|| {
        String::from("recovery operation record contains invalid canonical action bytes")
    })?;
    let retained = operation.intent.action_payload.as_deref().ok_or_else(|| {
        String::from("durable operation has no canonical action bytes for recovery")
    })?;
    if decoded.as_slice() != retained
        || format!("{:x}", sha2::Sha256::digest(&decoded)) != operation.intent.payload_digest
    {
        return Err(String::from(
            "recovery operation record canonical action bytes do not match the original digest",
        ));
    }
    object
        .get("state")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| String::from("recovery operation record omitted state"))
}

pub(super) fn authoritative_reconcile_state(
    payload: &Value,
    operation: &sts2_harness::StoredOperation,
) -> Result<(OperationState, DispatchStatus), String> {
    let value = payload
        .get("operation")
        .filter(|value| !value.is_null())
        .ok_or_else(|| String::from("reconcile response omitted the authoritative operation"))?;
    let object = value
        .as_object()
        .ok_or_else(|| String::from("reconcile operation record is not an object"))?;
    let state = operation_record(payload, operation)?;
    if state != "RECONCILED" {
        return Err(String::from(
            "reconcile operation record is not authoritative RECONCILED state",
        ));
    }
    let ticket = object
        .get("ticket")
        .and_then(Value::as_object)
        .ok_or_else(|| String::from("reconciled operation omitted its admission ticket"))?;
    if ticket.get("operation_id").and_then(Value::as_str)
        != Some(operation.intent.operation_id.as_str())
        || ticket.get("payload_digest").and_then(Value::as_str)
            != Some(operation.intent.payload_digest.as_str())
    {
        return Err(String::from(
            "reconciled operation ticket does not match the original operation",
        ));
    }
    match ticket.get("state").and_then(Value::as_str) {
        Some("SETTLED") => Ok((OperationState::Settled, DispatchStatus::Settled)),
        Some("REJECTED") => Ok((OperationState::Rejected, DispatchStatus::Rejected)),
        _ => Err(String::from(
            "reconciled operation ticket has no terminal authoritative state",
        )),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use sts2_harness::{
        DispatchStatus, ExecutionLineage, OperationIntent, OperationState, StoredOperation,
    };

    use super::{authoritative_reconcile_state, wire};

    const OPERATION_ID: &str = "11111111-1111-4111-8111-111111111111";
    const STATE_ID: &str = "22222222-2222-4222-8222-222222222222";
    const PAYLOAD_DIGEST: &str = "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a";
    const CATALOG_DIGEST: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

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
            b"{}".to_vec(),
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

    fn payload(ticket_state: &str) -> serde_json::Value {
        json!({
            "operation": {
                "operation_id": OPERATION_ID,
                "state": "RECONCILED",
                "payload_digest": PAYLOAD_DIGEST,
                "expected_boundary": {
                    "state_id": STATE_ID,
                    "generation": 0,
                    "catalog_digest": CATALOG_DIGEST
                },
                "action": {
                    "schema_digest": wire::RUNTIME_V3_SCHEMA_DIGEST,
                    "canonical_json_b64": "e30=",
                    "payload_digest": PAYLOAD_DIGEST
                },
                "ticket": {
                    "operation_id": OPERATION_ID,
                    "payload_digest": PAYLOAD_DIGEST,
                    "state": ticket_state
                }
            }
        })
    }

    #[test]
    fn authoritative_ticket_state_drives_recovery_outcome() -> Result<(), Box<dyn std::error::Error>>
    {
        let operation = operation()?;
        assert_eq!(
            authoritative_reconcile_state(&payload("SETTLED"), &operation)?,
            (OperationState::Settled, DispatchStatus::Settled)
        );
        assert_eq!(
            authoritative_reconcile_state(&payload("REJECTED"), &operation)?,
            (OperationState::Rejected, DispatchStatus::Rejected)
        );
        Ok(())
    }
}
