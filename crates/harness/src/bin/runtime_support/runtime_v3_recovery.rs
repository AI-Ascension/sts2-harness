// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use sts2_harness::{
    DispatchStatus, EpisodeObservation, OperationState, RecoveryError, RecoveryPort,
    TransitionReceipt, WaitOutcome,
};

use super::super::runtime_v3_telemetry::{ObservationSource, RecoveryKind};
use super::{RuntimeV3Port, parse, wire};

#[path = "runtime_v3_recovery_base64.rs"]
mod base64;
#[path = "runtime_v3_recovery_context.rs"]
mod context;
#[path = "runtime_v3_recovery_reconnect.rs"]
mod reconnect;
#[path = "runtime_v3_recovery_record.rs"]
mod record;
#[path = "runtime_v3_recovery_sideband.rs"]
mod sideband;
pub(super) use context::RecoveryContext;

impl RuntimeV3Port {
    /// Rehydrates the durable operation ledger after MCP startup and resolves each retained
    /// mutation before the runner can ask the provider for a new decision.
    pub(super) fn reconcile_pending_operations(&mut self) -> Result<(), String> {
        let Some(durable) = self.durable.clone() else {
            return Ok(());
        };
        let pending = durable.pending_operations()?;
        for operation in pending {
            let action_bytes = match operation.intent.action_payload.as_deref() {
                Some(bytes) => bytes,
                None => {
                    durable.mark_interrupted_unknown(
                        "legacy operation has no canonical action payload; recovery is blocked",
                    );
                    return Err(format!(
                        "cannot resume operation {} without its canonical action payload",
                        operation.intent.operation_id
                    ));
                }
            };
            let payload: Value = serde_json::from_slice(action_bytes).map_err(|_| {
                durable.mark_interrupted_unknown(
                    "durable operation canonical action payload is malformed",
                );
                format!(
                    "cannot resume operation {} with malformed canonical action payload",
                    operation.intent.operation_id
                )
            })?;
            let envelope = payload.as_object().ok_or_else(|| {
                durable.mark_interrupted_unknown(
                    "durable operation canonical action payload is not an object",
                );
                format!(
                    "cannot resume operation {} with a non-object canonical action payload",
                    operation.intent.operation_id
                )
            })?;
            if envelope.len() != 2
                || envelope.get("action_id").and_then(Value::as_str)
                    != Some(operation.intent.action_id.as_str())
            {
                durable.mark_interrupted_unknown(
                    "durable operation canonical action identity is inconsistent",
                );
                return Err(format!(
                    "cannot resume operation {} with inconsistent canonical action identity",
                    operation.intent.operation_id
                ));
            }
            let action_payload = envelope.get("action").ok_or_else(|| {
                durable.mark_interrupted_unknown(
                    "durable operation canonical action payload is incomplete",
                );
                format!(
                    "cannot resume operation {} with incomplete canonical action payload",
                    operation.intent.operation_id
                )
            })?;
            let action = super::parse::action_from_payload(
                &operation.intent.action_id,
                action_payload,
            )
            .map_err(|error| {
                durable.mark_interrupted_unknown(
                    "durable operation canonical action payload failed validation",
                );
                format!(
                    "cannot resume operation {} with invalid canonical action payload: {error}",
                    operation.intent.operation_id
                )
            })?;
            if operation.intent.action_kind.as_deref()
                != Some(super::wire::action_kind_name(action.kind()))
                || super::wire::canonical_action_digest(action.action_id(), action_payload)
                    .map_err(|error| {
                        durable.mark_interrupted_unknown(
                            "durable operation canonical action digest could not be calculated",
                        );
                        format!(
                            "cannot validate operation {} canonical action: {error}",
                            operation.intent.operation_id
                        )
                    })?
                    != operation.intent.payload_digest
            {
                durable.mark_interrupted_unknown(
                    "durable operation canonical action kind or digest does not match",
                );
                return Err(format!(
                    "cannot resume operation {} with mismatched canonical action identity",
                    operation.intent.operation_id
                ));
            }
            self.operations.insert(
                operation.intent.operation_id.clone(),
                super::OperationRecord {
                    state_id: operation.intent.state_id.clone(),
                    generation: operation.intent.generation,
                    action,
                },
            );
            let operation_id = operation.intent.operation_id.as_str();
            let receipt = self.reconcile(operation_id).map_err(|error| {
                format!("cannot reconcile retained operation {operation_id}: {error}")
            })?;
            match receipt.status() {
                DispatchStatus::Settled | DispatchStatus::Rejected | DispatchStatus::Cancelled => {}
                DispatchStatus::Accepted | DispatchStatus::Unknown => {
                    let sample = self
                        .poll_operation(operation_id, 1_000)
                        .map_err(|_| format!("retained operation {operation_id} did not settle"))?;
                    if !matches!(
                        sample.outcome(),
                        WaitOutcome::Successor | WaitOutcome::SameStateMutation
                    ) {
                        return Err(format!(
                            "retained operation {operation_id} remains unresolved"
                        ));
                    }
                }
            }
            let state = durable.operation_state(operation_id)?;
            if state.is_unresolved() {
                return Err(format!(
                    "retained operation {operation_id} remains in durable state {state:?}"
                ));
            }
        }
        durable.refresh_resume_boundary()?;
        Ok(())
    }
}

impl RecoveryPort for RuntimeV3Port {
    fn reobserve(&mut self) -> Result<EpisodeObservation, RecoveryError> {
        self.reconnect_for_recovery()?;
        let value = self
            .call_tool("sts2.reobserve", self.context(self.generation))
            .map_err(|_| RecoveryError::PortFailure)?;
        let parsed = parse::observation(&value, "reobserve_response", &self.config)
            .map_err(|_| RecoveryError::PortFailure)?;
        let observation = self
            .install(parsed)
            .map_err(|_| RecoveryError::PortFailure)?;
        let _ = self
            .telemetry
            .observation(ObservationSource::Reobserve, &observation);
        let _ = self.telemetry.recovery(
            RecoveryKind::Reobserve,
            None,
            self.reconnect_attempts,
            "success",
            None,
        );
        Ok(observation)
    }

    fn reconcile(&mut self, operation_id: &str) -> Result<TransitionReceipt, RecoveryError> {
        let record = self
            .operations
            .get(operation_id)
            .cloned()
            .ok_or(RecoveryError::InvalidOperation)?;
        let operation = self
            .durable_operation(operation_id)
            .map_err(|_| RecoveryError::PortFailure)?;
        let context = self
            .recovery_context
            .clone()
            .or_else(|| RecoveryContext::from_environment(&self.config).ok())
            .ok_or(RecoveryError::PortFailure)?;
        let operation_ref = context
            .operation_ref(&operation)
            .map_err(|_| RecoveryError::PortFailure)?;
        let lookup = self
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
        let lookup_state = record::operation_record(lookup_payload, &operation)
            .map_err(|_| RecoveryError::PortFailure)?;
        if lookup_payload
            .get("result")
            .and_then(|result| result.get("status"))
            .and_then(Value::as_str)
            != Some(lookup_state.as_str())
        {
            return Err(RecoveryError::PortFailure);
        }
        let resolved_state = match lookup_state.as_str() {
            "SETTLED" => (OperationState::Settled, DispatchStatus::Settled),
            "REJECTED" => (OperationState::Rejected, DispatchStatus::Rejected),
            "UNKNOWN" | "MAY_HAVE_BEEN_DISPATCHED" | "ACCEPTED" => {
                return Ok(TransitionReceipt::new(
                    operation_id,
                    record.action,
                    DispatchStatus::Unknown,
                    None,
                    None,
                    Some(String::from("recovery_required")),
                ));
            }
            _ => return Err(RecoveryError::PortFailure),
        };
        let reconcile = self
            .recovery_call_tool(
                "watchdog.operation_reconcile",
                "operation_reconcile_response",
                context
                    .reconcile_payload(&operation)
                    .map_err(|_| RecoveryError::PortFailure)?,
            )
            .map_err(|_| RecoveryError::PortFailure)?;
        let reconcile_payload = reconcile.get("payload").ok_or(RecoveryError::PortFailure)?;
        if reconcile_payload
            .get("result")
            .and_then(|result| result.get("status"))
            .and_then(Value::as_str)
            != Some("RECONCILED")
        {
            return Err(RecoveryError::PortFailure);
        }
        record::operation_record(reconcile_payload, &operation)
            .map_err(|_| RecoveryError::PortFailure)?;
        if let Some(durable) = &self.durable {
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
        super::recording::receipt(&receipt, record.generation, &self.telemetry);
        let _ = self.telemetry.recovery(
            RecoveryKind::Reconcile,
            Some(operation_id),
            self.reconnect_attempts,
            "success",
            None,
        );
        Ok(receipt)
    }

    fn release_lease(&mut self) -> Result<(), RecoveryError> {
        self.release_lease_inner()
            .map_err(|_| RecoveryError::PortFailure)
    }

    fn stop_episode(&mut self) -> Result<(), RecoveryError> {
        RecoveryPort::release_lease(self)
    }
}
