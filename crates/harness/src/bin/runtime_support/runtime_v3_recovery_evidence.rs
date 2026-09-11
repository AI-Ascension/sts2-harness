// SPDX-License-Identifier: MIT

use serde_json::Value;
use sts2_harness::{DispatchStatus, OperationState, StoredOperation};

pub(in super::super) fn authoritative_reconcile_state(
    payload: &Value,
    operation: &StoredOperation,
    original_context: &Value,
) -> Result<(OperationState, DispatchStatus), String> {
    let state = super::response_state(payload, operation, original_context)?
        .ok_or("missing operation is not terminal evidence")?;
    let record = &payload["operation"];
    let ticket = record
        .get("ticket")
        .filter(|value| value.is_object())
        .ok_or("terminal operation omitted its admission ticket")?;
    validate_ticket(ticket, operation, original_context)?;
    let ticket_state = ticket["state"]
        .as_str()
        .ok_or("terminal ticket omitted state")?;
    if !matches!(state.as_str(), "SETTLED" | "REJECTED" | "RECONCILED")
        || (state != "RECONCILED" && ticket_state != state)
    {
        return Err(String::from("operation and terminal ticket disagree"));
    }
    let witness = record
        .get("witness")
        .ok_or("operation omitted witness field")?;
    if payload.get("witness") != Some(witness) {
        return Err(String::from("reconcile witnesses disagree"));
    }
    if state == "RECONCILED" || ticket_state == "SETTLED" || !witness.is_null() {
        validate_witness(witness, ticket, operation, original_context)?;
    }
    match ticket_state {
        "SETTLED" => Ok((OperationState::Settled, DispatchStatus::Settled)),
        "REJECTED" => Ok((OperationState::Rejected, DispatchStatus::Rejected)),
        _ => Err(String::from("ticket has no terminal authoritative state")),
    }
}

fn validate_ticket(
    ticket: &Value,
    operation: &StoredOperation,
    original_context: &Value,
) -> Result<(), String> {
    if ticket["operation_id"].as_str() != Some(operation.intent.operation_id.as_str())
        || ticket["payload_digest"].as_str() != Some(operation.intent.payload_digest.as_str())
        || ["boot_id", "instance_incarnation", "lease_epoch"]
            .iter()
            .any(|field| ticket.get(field) != original_context.get(field))
        || ticket["host_fence_id"].as_str().is_none()
    {
        return Err(String::from(
            "terminal ticket does not match original operation authority",
        ));
    }
    Ok(())
}

fn validate_witness(
    witness: &Value,
    ticket: &Value,
    operation: &StoredOperation,
    original_context: &Value,
) -> Result<(), String> {
    if !witness.is_object()
        || witness["operation_id"].as_str() != Some(operation.intent.operation_id.as_str())
        || witness["payload_digest"].as_str() != Some(operation.intent.payload_digest.as_str())
        || ["boot_id", "instance_incarnation"]
            .iter()
            .any(|field| witness.get(field) != original_context.get(field))
        || witness.get("host_fence_id") != ticket.get("host_fence_id")
        || !witness["generation"]
            .as_u64()
            .is_some_and(|value| value > operation.intent.generation)
        || !matches!(
            witness["source"].as_str(),
            Some("host_game_thread" | "host_receipt" | "authoritative_reobserve")
        )
    {
        return Err(String::from(
            "effect witness does not prove this operation under its original authority",
        ));
    }
    Ok(())
}
