// SPDX-License-Identifier: MIT

use sts2_harness::RecoveryError;

use super::super::super::mcp::McpProcess;
use super::super::super::runtime_v3_telemetry::RecoveryKind;
use super::{RuntimeV3Port, wire};

impl RuntimeV3Port {
    // Reconnect only for recovery reads, never to repeat a dispatch. The episode ledger and
    // configured lease/session survive replacement of a failed MCP transport.
    pub(super) fn reconnect_for_recovery(&mut self) -> Result<(), RecoveryError> {
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
}
