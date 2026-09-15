// SPDX-License-Identifier: MIT
use super::*;
use sts2_harness::game_information::{
    LookupError, LookupMcpContext, LookupMcpPort, call_lookup_mcp, call_capabilities_mcp,
};

impl LookupMcpPort for RuntimeV3Port {
    fn information_correlation(&self) -> Result<String, LookupError> {
        if self.mcp.is_none() {
            return Err(LookupError::Transport);
        }
        Ok(self.next_rpc_id.to_string())
    }
    fn information_capabilities(&mut self) -> Result<(String, Vec<u8>), LookupError> {
        let correlation = self.information_correlation()?;
        let id = self.next_rpc_id;
        self.next_rpc_id = id.checked_add(1).ok_or(LookupError::Bounds)?;
        let context = LookupMcpContext {
            instance_id: self.config.instance_id.clone(), mcp_session_id: self.config.mcp_session_id.clone(),
            lease_id: self.config.lease_id.clone(), lease_epoch: self.config.lease_epoch,
        };
        let bytes = call_capabilities_mcp(&context,id,|id,args| {
            wire::rpc_call_catalog_read(self.mcp.as_mut().ok_or(LookupError::Transport)?,
                id,"tools/call",args).map_err(|_|LookupError::Transport)
        })?;
        Ok((correlation,bytes))
    }
    fn call_information(&mut self, tool: &str, request: &Value) -> Result<Vec<u8>, LookupError> {
        let context = LookupMcpContext {
            instance_id: self.config.instance_id.clone(),
            mcp_session_id: self.config.mcp_session_id.clone(),
            lease_id: self.config.lease_id.clone(),
            lease_epoch: self.config.lease_epoch,
        };
        let expected_correlation = self.next_rpc_id.to_string();
        if request["correlation_id"].as_str() != Some(expected_correlation.as_str()) {
            return Err(LookupError::Scope);
        }
        self.next_rpc_id = self.next_rpc_id.checked_add(1).ok_or(LookupError::Bounds)?;
        call_lookup_mcp(&context, tool, request, |id, arguments| {
            wire::rpc_call_catalog_read(
                self.mcp.as_mut().ok_or(LookupError::Transport)?,
                id,
                "tools/call",
                arguments,
            )
            .map_err(|_| LookupError::Transport)
        })
    }
}
