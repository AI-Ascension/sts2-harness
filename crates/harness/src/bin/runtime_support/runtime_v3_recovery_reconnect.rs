// SPDX-License-Identifier: MIT

use sts2_harness::RecoveryError;

use super::super::super::mcp::McpProcess;
use super::super::super::runtime_v3_telemetry::RecoveryKind;
use super::super::{RuntimeV3Port, wire};
use super::map_initialization_error;

impl RuntimeV3Port {
    // Reconnect only for recovery reads, never to repeat a dispatch. The episode ledger and
    // configured lease/session survive replacement of a failed MCP transport.
    pub(super) fn reconnect_for_recovery(&mut self) -> Result<(), RecoveryError> {
        if !self.allocated || self.released {
            return Err(RecoveryError::PortFailure);
        }
        let normal_ready = self.mcp.as_mut().is_some_and(|mcp| !mcp.refresh_closed());
        let expert_ready = !self.is_expert_profile()
            || self
                .expert_mcp
                .as_mut()
                .is_some_and(|mcp| !mcp.refresh_closed());
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
        let mut mcp = if self.is_expert_profile() {
            McpProcess::spawn_profile(&self.config, normal_profile)
        } else {
            McpProcess::spawn_with_cancellation(&self.config, &self.cancellation)
        }
        .map_err(|_| RecoveryError::PortFailure)?;
        wire::initialize_mcp_profile_classified(&mut mcp, normal_profile)
            .map_err(map_initialization_error)?;
        self.mcp = Some(mcp);
        if self.is_expert_profile() {
            let mut expert = McpProcess::spawn_profile(&self.config, "runtime-v4-expert")
                .map_err(|_| RecoveryError::PortFailure)?;
            wire::initialize_mcp_profile_classified(&mut expert, "runtime-v4-expert")
                .map_err(map_initialization_error)?;
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
