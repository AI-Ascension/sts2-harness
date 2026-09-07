// SPDX-License-Identifier: MIT

use serde_json::json;
use sts2_harness::{
    DispatchStatus, EpisodeLegalAction, EpisodeObservation, OperationState, RecoveryError,
    RecoveryPort, TransitionReceipt, WaitOutcome,
};

use super::super::mcp::McpProcess;
use super::super::runtime_v3_telemetry::{ObservationSource, RecoveryKind};
use super::{RuntimeV3Port, parse, wire};

impl RuntimeV3Port {
    // Reconnect only for recovery reads, never to repeat a dispatch. The episode ledger and
    // configured lease/session survive replacement of a failed MCP transport.
    fn reconnect_for_recovery(&mut self) -> Result<(), RecoveryError> {
        if !self.allocated || self.released {
            return Err(RecoveryError::PortFailure);
        }
        if self.mcp.as_ref().is_some_and(|mcp| !mcp.is_closed()) {
            return Ok(());
        }
        if self.reconnect_attempts >= 2 {
            return Err(RecoveryError::PortFailure);
        }
        self.reconnect_attempts += 1;
        if let Some(mut previous) = self.mcp.take() {
            previous.close().map_err(|_| RecoveryError::PortFailure)?;
        }
        let mut mcp = McpProcess::spawn(&self.config).map_err(|_| RecoveryError::PortFailure)?;
        wire::initialize_mcp(&mut mcp).map_err(|_| RecoveryError::PortFailure)?;
        self.mcp = Some(mcp);
        let _ = self.telemetry.recovery(
            RecoveryKind::Reconnect,
            None,
            self.reconnect_attempts,
            "success",
            None,
        );
        Ok(())
    }

    /// Rehydrates the durable operation ledger after MCP startup and resolves each retained
    /// mutation before the runner can ask the provider for a new decision.
    pub(super) fn reconcile_pending_operations(&mut self) -> Result<(), String> {
        let Some(durable) = self.durable.clone() else {
            return Ok(());
        };
        let pending = durable.pending_operations()?;
        for operation in pending {
            let action_kind = super::wire::action_kind_for_id(&operation.intent.action_id)
                .ok_or_else(|| {
                    format!(
                        "cannot resume operation {} with an unknown semantic action kind",
                        operation.intent.operation_id
                    )
                })?;
            let action = EpisodeLegalAction::new(operation.intent.action_id.clone(), action_kind)
                .map_err(|error| format!("cannot resume operation identity: {error}"))?;
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
        self.reconnect_for_recovery()?;
        let value = self
            .call_tool(
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
            "recover_response",
            &self.config,
            operation_id,
            record.generation,
            record.action,
        )
        .map_err(|_| RecoveryError::PortFailure)?;
        self.install_response(&value, "recover_response")
            .map_err(|_| RecoveryError::PortFailure)?;
        if let Some(durable) = &self.durable {
            match receipt.status() {
                DispatchStatus::Settled => durable
                    .reconcile_response(operation_id, OperationState::Settled, &value)
                    .map_err(|_| RecoveryError::PortFailure)?,
                DispatchStatus::Rejected | DispatchStatus::Cancelled => durable
                    .reconcile_response(operation_id, OperationState::Rejected, &value)
                    .map_err(|_| RecoveryError::PortFailure)?,
                DispatchStatus::Accepted | DispatchStatus::Unknown => {}
            }
        }
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
