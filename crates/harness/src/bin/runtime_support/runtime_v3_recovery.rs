// SPDX-License-Identifier: MIT

use serde_json::json;
use sts2_harness::{
    BarrierError, BarrierPort, EpisodeObservation, RecoveryError, RecoveryPort, TransitionReceipt,
    WaitSample,
};

use super::{RuntimeV3Port, parse};

impl BarrierPort for RuntimeV3Port {
    fn wait_for_transition(
        &mut self,
        operation_id: &str,
        wait_for_millis: u32,
    ) -> Result<WaitSample, BarrierError> {
        let value = self
            .call_tool(
                "sts2.wait_for_transition",
                json!({
                    "instance_id": self.config.instance_id,
                    "mcp_session_id": self.config.mcp_session_id,
                    "lease_id": self.config.lease_id,
                    "lease_epoch": self.config.lease_epoch,
                    "generation": self.generation,
                    "operation_id": operation_id,
                    "wait_for_millis": wait_for_millis
                }),
            )
            .map_err(|_| BarrierError::PortFailure)?;
        let expected_generation = self
            .operations
            .get(operation_id)
            .map_or(self.generation, |record| record.generation);
        let sample = parse::wait_sample(&value, &self.config, operation_id, expected_generation)
            .map_err(|_| BarrierError::PortFailure)?;
        self.install_response(&value, "wait_response")
            .map_err(|_| BarrierError::PortFailure)?;
        Ok(sample)
    }
}

impl RecoveryPort for RuntimeV3Port {
    fn reobserve(&mut self) -> Result<EpisodeObservation, RecoveryError> {
        let value = self
            .call_tool("sts2.reobserve", self.context(self.generation))
            .map_err(|_| RecoveryError::PortFailure)?;
        let parsed = parse::observation(&value, "reobserve_response", &self.config)
            .map_err(|_| RecoveryError::PortFailure)?;
        Ok(self.install(parsed))
    }

    fn reconcile(&mut self, operation_id: &str) -> Result<TransitionReceipt, RecoveryError> {
        let record = self
            .operations
            .get(operation_id)
            .cloned()
            .ok_or(RecoveryError::InvalidOperation)?;
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
