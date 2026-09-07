// SPDX-License-Identifier: MIT

use serde_json::json;
use sts2_harness::{EpisodeObservation, RecoveryError, RecoveryPort, TransitionReceipt};

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
        let normal_ready = self.mcp.as_ref().is_some_and(|mcp| !mcp.is_closed());
        let expert_ready = !self.is_expert_profile()
            || self.expert_mcp.as_ref().is_some_and(|mcp| !mcp.is_closed());
        if normal_ready && expert_ready {
            return Ok(());
        }
        if self.reconnect_attempts >= 2 {
            return Err(RecoveryError::PortFailure);
        }
        self.reconnect_attempts += 1;
        if let Some(mut previous) = self.mcp.take() {
            previous.close().map_err(|_| RecoveryError::PortFailure)?;
        }
        if let Some(mut previous) = self.expert_mcp.take() {
            previous.close().map_err(|_| RecoveryError::PortFailure)?;
        }
        let normal_profile = if self.is_expert_profile() {
            "runtime-v3-gameplay"
        } else {
            self.config.runtime_profile.as_str()
        };
        let mut mcp = McpProcess::spawn_profile(&self.config, normal_profile)
            .map_err(|_| RecoveryError::PortFailure)?;
        wire::initialize_mcp_profile(&mut mcp, normal_profile)
            .map_err(|_| RecoveryError::PortFailure)?;
        self.mcp = Some(mcp);
        if self.is_expert_profile() {
            let mut expert = McpProcess::spawn_profile(&self.config, "runtime-v4-expert")
                .map_err(|_| RecoveryError::PortFailure)?;
            wire::initialize_mcp_profile(&mut expert, "runtime-v4-expert")
                .map_err(|_| RecoveryError::PortFailure)?;
            self.expert_mcp = Some(expert);
        }
        let _ = self.telemetry.recovery(
            RecoveryKind::Reconnect,
            None,
            self.reconnect_attempts,
            "success",
            None,
        );
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
        let baseline = self.install(parsed);
        let observation = if self.is_expert_profile() {
            self.compose_current_observation(baseline)
                .map_err(|_| RecoveryError::PortFailure)?
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
