// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use sts2_harness::{DispatchStatus, RecoveryError, TransitionReceipt};

use super::super::super::runtime_v3_telemetry::RecoveryKind;
use super::super::{OperationRecord, RuntimeV3Port};
use super::{RecoveryContext, record};

// A terminal historical witness is already authoritative.  The follow-up gameplay request is
// only a retained-state read needed to build a usable continuation receipt; it must not add the
// ordinary 120-second transition barrier to the recovery deadline.  If the host cannot return a
// fresh witness immediately, fail closed and leave the durable operation unresolved.
const RETAINED_WITNESS_WAIT_MILLIS: u32 = 1;

pub(super) fn reconcile_durable_operation(
    port: &mut RuntimeV3Port,
    operation_id: &str,
    record: OperationRecord,
) -> Result<TransitionReceipt, RecoveryError> {
    let operation = port
        .durable_operation(operation_id)
        .map_err(|_| RecoveryError::PortFailure)?;
    let operation_ref =
        RecoveryContext::operation_ref(&operation).map_err(|_| RecoveryError::PortFailure)?;
    port.ensure_recovery_sideband()
        .map_err(|_| RecoveryError::PortFailure)?;
    let context = port
        .recovery_context
        .clone()
        .ok_or(RecoveryError::PortFailure)?;
    let lookup = port
        .recovery_call_tool(
            "watchdog.operation_lookup",
            "operation_lookup_response",
            json!({"operation": operation_ref, "lookup_scope": "historical_read"}),
        )
        .map_err(|_| RecoveryError::PortFailure)?;
    let lookup_payload = lookup.get("payload").ok_or(RecoveryError::PortFailure)?;
    if lookup_payload.get("mutation_authorized") != Some(&Value::Bool(false)) {
        return Err(RecoveryError::PortFailure);
    }
    let original_context = &operation_ref["original_context"];
    let lookup_state = record::response_state(lookup_payload, &operation, original_context)
        .map_err(|_| RecoveryError::PortFailure)?;
    if lookup_state.is_none() {
        return Ok(unknown_receipt(operation_id, record.action));
    }
    let reconcile = port
        .recovery_call_tool(
            "watchdog.operation_reconcile",
            "operation_reconcile_response",
            context
                .reconcile_payload(&operation)
                .map_err(|_| RecoveryError::PortFailure)?,
        )
        .map_err(|_| RecoveryError::PortFailure)?;
    let reconcile_payload = reconcile.get("payload").ok_or(RecoveryError::PortFailure)?;
    let state = record::response_state(reconcile_payload, &operation, original_context)
        .map_err(|_| RecoveryError::PortFailure)?;
    if !matches!(
        state.as_deref(),
        Some("SETTLED" | "REJECTED" | "RECONCILED")
    ) {
        return Ok(unknown_receipt(operation_id, record.action));
    }
    let resolved_state =
        record::authoritative_reconcile_state(reconcile_payload, &operation, original_context)
            .map_err(|_| RecoveryError::PortFailure)?;
    if resolved_state.1 == DispatchStatus::Settled {
        let current_authority = port
            .recovery_authority
            .as_ref()
            .ok_or(RecoveryError::PortFailure)?;
        if !same_host(original_context, current_authority) {
            // A different host incarnation is a reconstruction boundary, not an in-place
            // continuation.  Keep the durable operation unresolved until an explicitly approved
            // reconstruction can establish a new action boundary.
            return Err(RecoveryError::Terminal);
        }
        let witness_generation = reconcile_payload["operation"]["witness"]["generation"]
            .as_u64()
            .ok_or(RecoveryError::PortFailure)?;
        port.reconnect_for_recovery()?;
        let receipt = port
            .recovery_wait_for_settled_operation(
                operation_id,
                witness_generation,
                RETAINED_WITNESS_WAIT_MILLIS,
            )
            .map_err(|_| RecoveryError::PortFailure)?;
        if let Some(durable) = &port.durable {
            durable
                .reconcile_response(operation_id, resolved_state.0, &reconcile)
                .map_err(|_| RecoveryError::PortFailure)?;
        }
        super::super::recording::receipt(&receipt, record.generation, &port.telemetry);
        let _ = port.telemetry.recovery(
            RecoveryKind::Reconcile,
            Some(operation_id),
            port.reconnect_attempts,
            "success",
            None,
        );
        return Ok(receipt);
    }
    if let Some(durable) = &port.durable {
        durable
            .reconcile_response(operation_id, resolved_state.0, &reconcile)
            .map_err(|_| RecoveryError::PortFailure)?;
    }
    let receipt = TransitionReceipt::new(
        operation_id,
        record.action,
        resolved_state.1,
        None,
        None,
        None,
    );
    super::super::recording::receipt(&receipt, record.generation, &port.telemetry);
    let _ = port.telemetry.recovery(
        RecoveryKind::Reconcile,
        Some(operation_id),
        port.reconnect_attempts,
        "success",
        None,
    );
    Ok(receipt)
}

fn same_host(
    original: &Value,
    current: &super::super::allocation_context::RecoveryAuthority,
) -> bool {
    // Gateway boot, lease and fence may rotate without replacing this host.
    // In-place continuation still requires all three host identity namespaces.
    original["deployment_id"].as_str() == Some(current.deployment_id.as_str())
        && original["instance_id"].as_str() == Some(current.instance_id.as_str())
        && original["instance_incarnation"].as_str() == Some(current.instance_incarnation.as_str())
}

fn unknown_receipt(
    operation_id: &str,
    action: sts2_harness::EpisodeLegalAction,
) -> TransitionReceipt {
    TransitionReceipt::new(
        operation_id,
        action,
        DispatchStatus::Unknown,
        None,
        None,
        Some(String::from("recovery_required")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_identity_requires_all_namespaces_but_allows_fresh_authority() {
        let current = super::super::super::allocation_context::RecoveryAuthority {
            deployment_id: "deployment".into(),
            instance_id: "instance".into(),
            instance_incarnation: "incarnation".into(),
            boot_id: "new-boot".into(),
            authority_generation: 2,
            lease_id: "new-lease".into(),
            lease_epoch: 2,
            current_fence: json!({"host_fence_id":"new-fence"}),
        };
        let original = json!({
            "deployment_id":"deployment", "instance_id":"instance",
            "instance_incarnation":"incarnation", "boot_id":"old-boot",
            "lease_id":"old-lease", "lease_epoch":1
        });
        assert!(same_host(&original, &current));
        for field in ["deployment_id", "instance_id", "instance_incarnation"] {
            let mut other = original.clone();
            other[field] = json!("different");
            assert!(!same_host(&other, &current), "accepted changed {field}");
            other[field] = Value::Null;
            assert!(!same_host(&other, &current), "accepted absent {field}");
        }
    }
}
