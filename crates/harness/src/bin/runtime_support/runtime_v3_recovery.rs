// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use sts2_harness::{
    DispatchStatus, EpisodeObservation, RecoveryError, RecoveryPort, TransitionReceipt,
};

use super::super::runtime_v3_telemetry::{ObservationSource, RecoveryKind};
use super::{RuntimeV3Port, parse, wire};

#[path = "runtime_v3_recovery_context.rs"]
mod context;
#[path = "runtime_v3_recovery_reconnect.rs"]
mod reconnect;
#[path = "runtime_v3_recovery_record.rs"]
mod record;
#[path = "runtime_v3_recovery_sideband.rs"]
mod sideband;
pub(super) use context::RecoveryContext;

include!("runtime_v3_recovery_pending.rs");

impl RecoveryPort for RuntimeV3Port {
    fn reobserve(&mut self) -> Result<EpisodeObservation, RecoveryError> {
        self.reconnect_for_recovery()?;
        let value = match self.call_tool_classified("sts2.reobserve", self.context(self.generation))
        {
            Ok(value) => value,
            Err(super::RuntimeV3ToolError::Transient(_)) => {
                return Err(RecoveryError::PortFailure);
            }
            Err(super::RuntimeV3ToolError::Terminal(_)) => return Err(RecoveryError::Terminal),
        };
        let response_text = self
            .last_response_text
            .clone()
            .ok_or(RecoveryError::Terminal)?;
        let parsed = parse::observation_with_text(
            &value,
            &response_text,
            "reobserve_response",
            &self.config,
        )
        .map_err(|_| RecoveryError::Terminal)?;
        let baseline = self.install(parsed).map_err(|_| RecoveryError::Terminal)?;
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

        if !self.historical_recovery_enabled() {
            self.reconnect_for_recovery()?;
            if self.uses_expert_transport(&record.action, &record.payload) {
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
            if matches!(
                receipt.status(),
                DispatchStatus::Settled | DispatchStatus::Rejected | DispatchStatus::Cancelled
            ) && let Some(durable) = &self.durable
            {
                durable
                    .clear_resume_boundary()
                    .map_err(|_| RecoveryError::PortFailure)?;
            }
            if matches!(
                receipt.status(),
                DispatchStatus::Settled | DispatchStatus::Rejected | DispatchStatus::Cancelled
            ) {
                self.install_response(&value, &response_text, "recover_response")
                    .map_err(|_| RecoveryError::PortFailure)?;
            }
            let receipt = if self.is_expert_profile() {
                self.compose_receipt_after(receipt)
                    .map_err(|_| RecoveryError::PortFailure)?
            } else {
                receipt
            };
            let payload_digest = self
                .durable
                .as_ref()
                .map_or_else(
                    || Ok(String::new()),
                    |durable| durable.operation_payload_digest(operation_id),
                )
                .map_err(|_| RecoveryError::PortFailure)?;
            self.record_reconciled_durable_receipt(operation_id, &payload_digest, &receipt, &value)
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

        let operation = self
            .durable_operation(operation_id)
            .map_err(|_| RecoveryError::PortFailure)?;
        let context = match (&self.recovery_authority, self.recovery_context.clone()) {
            (Some(_), Some(context)) => context,
            (Some(_), None) => return Err(RecoveryError::PortFailure),
            (None, context) => context
                .or_else(|| RecoveryContext::from_environment(&self.config).ok())
                .ok_or(RecoveryError::PortFailure)?,
        };
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
        let original_context = &operation_ref["original_context"];
        let lookup_state = record::response_state(lookup_payload, &operation, original_context)
            .map_err(|_| RecoveryError::PortFailure)?;
        if lookup_state.is_none() {
            return Ok(unknown_receipt(operation_id, record.action));
        }
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
        if let Some(durable) = &self.durable {
            // The old pre-dispatch checkpoint is no longer an admissible fresh observation after
            // authoritative settlement. Clear it only after the sideband evidence has been
            // validated, then let a read-only reobserve install the post-reconcile boundary.
            durable
                .clear_resume_boundary()
                .map_err(|_| RecoveryError::PortFailure)?;
        }
        let after = self.reobserve().map_err(|_| RecoveryError::PortFailure)?;
        if let Some(durable) = &self.durable {
            durable
                .reconcile_response(operation_id, resolved_state.0, &reconcile)
                .map_err(|_| RecoveryError::PortFailure)?;
        }
        let receipt = TransitionReceipt::new(
            operation_id,
            record.action,
            resolved_state.1,
            (resolved_state.1 == DispatchStatus::Settled).then_some(after),
            (resolved_state.1 == DispatchStatus::Settled)
                .then(|| String::from("authoritative_reobserve")),
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
