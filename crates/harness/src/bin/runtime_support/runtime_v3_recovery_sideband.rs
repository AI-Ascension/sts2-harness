// SPDX-License-Identifier: MIT

use serde_json::{Value, json};

use super::super::super::mcp::McpProcess;
use super::{RecoveryContext, RuntimeV3Port, wire};

impl RuntimeV3Port {
    pub(super) fn ensure_recovery_sideband(&mut self) -> Result<(), String> {
        if self.recovery.as_ref().is_some_and(|mcp| !mcp.is_closed()) {
            return Ok(());
        }
        if let Some(mut previous) = self.recovery.take() {
            previous.close()?;
        }
        let authority = self.recovery_authority.as_ref().ok_or_else(|| {
            String::from("validated allocation recovery authority is unavailable")
        })?;
        let context = RecoveryContext::from_authority(&self.config, authority)?;
        let authority_value = json!({
            "deployment_id": authority.deployment_id,
            "instance_id": authority.instance_id,
            "instance_incarnation": authority.instance_incarnation,
            "boot_id": authority.boot_id,
            "authority_generation": authority.authority_generation,
            "lease_id": authority.lease_id,
            "lease_epoch": authority.lease_epoch,
            "current_fence": authority.current_fence,
        });
        let mut mcp = McpProcess::spawn_recovery(
            &self.config,
            &context.instance_id,
            &context.lease_id,
            context.lease_epoch,
            &authority_value,
            &self.cancellation,
        )?;
        if let Err(error) = wire::initialize_recovery_mcp(&mut mcp) {
            let _ = mcp.close();
            return Err(error);
        }
        self.recovery_context = Some(context);
        self.recovery = Some(mcp);
        Ok(())
    }

    pub(super) fn recovery_call_tool(
        &mut self,
        name: &str,
        expected_kind: &str,
        payload: Value,
    ) -> Result<Value, String> {
        self.ensure_recovery_sideband()?;
        let id = self.recovery_rpc_id;
        self.recovery_rpc_id = self
            .recovery_rpc_id
            .checked_add(1)
            .ok_or_else(|| String::from("recovery MCP request identity exhausted"))?;
        let session = self
            .recovery_context
            .as_ref()
            .ok_or_else(|| String::from("recovery context is unavailable"))?
            .mcp_session_id
            .clone();
        let mcp = self
            .recovery
            .as_mut()
            .ok_or_else(|| String::from("recovery MCP process is unavailable"))?;
        wire::recovery_call(mcp, id, &session, name, expected_kind, payload)
    }

    pub(super) fn durable_operation(
        &self,
        operation_id: &str,
    ) -> Result<sts2_harness::StoredOperation, String> {
        let durable = self.durable.as_ref().ok_or_else(|| {
            String::from("historical recovery requires the durable operation ledger")
        })?;
        durable
            .pending_operations()?
            .into_iter()
            .find(|operation| operation.intent.operation_id == operation_id)
            .ok_or_else(|| format!("durable operation {operation_id} is not pending"))
    }
}
