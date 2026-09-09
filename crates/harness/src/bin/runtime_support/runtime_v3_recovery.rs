// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use sts2_harness::{
    DispatchStatus, EpisodeObservation, RecoveryError, RecoveryPort, TransitionReceipt,
};

use super::super::runtime_v3_telemetry::{ObservationSource, RecoveryKind};
use super::{RuntimeV3Port, parse, wire};

#[path = "runtime_v3_recovery_context.rs"]
mod context;
#[path = "runtime_v3_recovery_durable_reconcile.rs"]
mod durable_reconcile;
#[path = "runtime_v3_recovery_reconnect.rs"]
mod reconnect;
#[path = "runtime_v3_recovery_record.rs"]
mod record;
#[path = "runtime_v3_recovery_sideband.rs"]
mod sideband;
pub(super) use context::RecoveryContext;
use durable_reconcile::reconcile_durable_operation;

fn map_initialization_error(error: wire::RpcFailure) -> RecoveryError {
    if error.is_transient() {
        RecoveryError::PortFailure
    } else {
        RecoveryError::Terminal
    }
}

impl RuntimeV3Port {
    /// Rehydrates the durable operation ledger after MCP startup and resolves each retained
    /// mutation before the runner can ask the provider for a new decision.
    pub(super) fn reconcile_pending_operations(&mut self) -> Result<(), String> {
        let Some(durable) = self.durable.clone() else {
            return Ok(());
        };
        let pending = durable.pending_operations()?;
        for operation in pending {
            if operation.intent.original_context_raw.is_none() {
                return Err(durable.quarantine_failure(
                    format!(
                        "cannot resume operation {} without its original recovery context",
                        operation.intent.operation_id
                    ),
                    "durable operation has no immutable original recovery context evidence",
                ));
            }
            let action_bytes = match operation.intent.action_payload.as_deref() {
                Some(bytes) => bytes,
                None => {
                    return Err(durable.quarantine_failure(
                        format!(
                            "cannot resume operation {} without its canonical action payload",
                            operation.intent.operation_id
                        ),
                        "legacy operation has no canonical action payload; recovery is blocked",
                    ));
                }
            };
            let payload: Value = serde_json::from_slice(action_bytes).map_err(|_| {
                durable.quarantine_failure(
                    format!(
                        "cannot resume operation {} with malformed canonical action payload",
                        operation.intent.operation_id
                    ),
                    "durable operation canonical action payload is malformed",
                )
            })?;
            let envelope = payload.as_object().ok_or_else(|| {
                durable.quarantine_failure(
                    format!(
                        "cannot resume operation {} with a non-object canonical action payload",
                        operation.intent.operation_id
                    ),
                    "durable operation canonical action payload is not an object",
                )
            })?;
            if envelope.len() != 2
                || envelope.get("action_id").and_then(Value::as_str)
                    != Some(operation.intent.action_id.as_str())
            {
                return Err(durable.quarantine_failure(
                    format!(
                        "cannot resume operation {} with inconsistent canonical action identity",
                        operation.intent.operation_id
                    ),
                    "durable operation canonical action identity is inconsistent",
                ));
            }
            let action_payload = envelope.get("action").ok_or_else(|| {
                durable.quarantine_failure(
                    format!(
                        "cannot resume operation {} with incomplete canonical action payload",
                        operation.intent.operation_id
                    ),
                    "durable operation canonical action payload is incomplete",
                )
            })?;
            let action =
                super::parse::action_from_payload(&operation.intent.action_id, action_payload)
                    .map_err(|error| {
                        durable.quarantine_failure(
                    format!(
                        "cannot resume operation {} with invalid canonical action payload: {error}",
                        operation.intent.operation_id
                    ),
                    "durable operation canonical action payload failed validation",
                )
                    })?;
            if operation.intent.action_kind.as_deref()
                != Some(super::wire::action_kind_name(action.kind()))
                || super::wire::canonical_action_digest(
                    operation.intent.action_id.as_str(),
                    action_payload,
                )
                .map_err(|error| {
                    durable.quarantine_failure(
                        format!(
                            "cannot validate operation {} canonical action: {error}",
                            operation.intent.operation_id
                        ),
                        "durable operation canonical action digest could not be calculated",
                    )
                })? != operation.intent.payload_digest
            {
                return Err(durable.quarantine_failure(
                    format!(
                        "cannot resume operation {} with mismatched canonical action identity",
                        operation.intent.operation_id
                    ),
                    "durable operation canonical action kind or digest does not match",
                ));
            }
            self.operations.insert(
                operation.intent.operation_id.clone(),
                super::OperationRecord {
                    state_id: operation.intent.state_id.clone(),
                    generation: operation.intent.generation,
                    action,
                    payload: action_payload.clone(),
                },
            );
            let operation_id = operation.intent.operation_id.as_str();
            let receipt = self.reconcile(operation_id).map_err(|error| {
                format!("cannot reconcile retained operation {operation_id}: {error}")
            })?;
            match receipt.status() {
                DispatchStatus::Settled | DispatchStatus::Rejected | DispatchStatus::Cancelled => {}
                DispatchStatus::Accepted | DispatchStatus::Unknown => {
                    return Err(format!(
                        "retained operation {operation_id} remains unresolved"
                    ));
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
        let (value, response_text) = match self
            .call_tool_classified_with_text("sts2.reobserve", self.context(self.generation))
        {
            Ok(value) => value,
            Err(super::RuntimeV3ToolError::Transient(_)) => return Err(RecoveryError::PortFailure),
            Err(super::RuntimeV3ToolError::Terminal(_)) => return Err(RecoveryError::Terminal),
        };
        let parsed = parse::observation_with_text(
            &value,
            &response_text,
            "reobserve_response",
            &self.config,
        )
        .map_err(|_| RecoveryError::Terminal)?;
        let baseline = self
            .install(parsed)
            .map_err(|_| RecoveryError::PortFailure)?;
        let observation = if self.is_expert_profile() {
            self.compose_current_observation_recovery(baseline)
                .map_err(|error| match error {
                    super::RuntimeV3ToolError::Transient(_) => RecoveryError::PortFailure,
                    super::RuntimeV3ToolError::Terminal(_) => RecoveryError::Terminal,
                })?
        } else {
            baseline
        };
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
        if self.durable.is_some() {
            return reconcile_durable_operation(self, operation_id, record);
        }
        self.reconnect_for_recovery()?;
        if self.is_expert_profile() && record.action.kind() == sts2_harness::ActionKind::UsePotion {
            let receipt = self
                .reconcile_expert_operation(operation_id)
                .map_err(|_| RecoveryError::PortFailure)?;
            super::recording::receipt(&receipt, record.generation, &self.telemetry);
            let _ = self.telemetry.recovery(
                RecoveryKind::Reconcile,
                Some(operation_id),
                self.reconnect_attempts,
                "success",
                None,
            );
            return Ok(receipt);
        }
        let (value, response_text) = self
            .call_tool_with_text(
                "sts2.recover",
                json!({
                    "instance_id": self.config.instance_id,
                    "mcp_session_id": self.config.mcp_session_id,
                    "lease_id": self.config.lease_id,
                    "lease_epoch": self.config.lease_epoch,
                    "generation": self.generation,
                    "recovery_kind": "reconcile",
                    "operation_id": operation_id
                }),
            )
            .map_err(|_| RecoveryError::PortFailure)?;
        let receipt = parse::receipt(
            &value,
            &response_text,
            "recover_response",
            &self.config,
            operation_id,
            record.generation,
            record.action,
        )
        .map_err(|_| RecoveryError::PortFailure)?;
        self.install_response(&value, &response_text, "recover_response")
            .map_err(|_| RecoveryError::PortFailure)?;
        let receipt = if self.is_expert_profile() {
            self.compose_receipt_after(receipt)
                .map_err(|_| RecoveryError::PortFailure)?
        } else {
            receipt
        };
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
