// SPDX-License-Identifier: MIT

use sts2_harness::{ShutdownError, ShutdownPort};

use super::RuntimeV3Port;

impl ShutdownPort for RuntimeV3Port {
    fn mark_episode_completed(&mut self) {
        self.episode_completed = true;
    }

    fn release_lease(&mut self) -> Result<(), ShutdownError> {
        self.release_lease_inner()
            .map_err(|_| ShutdownError::ReleaseFailed)
    }

    fn close_mcp(&mut self) -> Result<(), ShutdownError> {
        self.close_mcp_processes()
            .map_err(|_| ShutdownError::McpCloseFailed)
    }

    fn close_gateway(&mut self) -> Result<(), ShutdownError> {
        if self.allocated
            && !self.released
            && !(self.continuation_adopted && !self.continuation_boundary_verified)
        {
            return Err(ShutdownError::GatewayCloseFailed);
        }
        Ok(())
    }
}
