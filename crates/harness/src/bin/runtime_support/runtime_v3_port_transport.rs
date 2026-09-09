// SPDX-License-Identifier: MIT

use serde_json::{Value, json};

use super::super::mcp::McpProcess;
use super::super::runtime_v3_wire as wire;
use super::{RuntimeV3Port, RuntimeV3ToolError, classify_mcp_error};

impl RuntimeV3Port {
    pub(super) fn context(&self, generation: u64) -> Value {
        json!({
            "instance_id": self.config.instance_id,
            "mcp_session_id": self.config.mcp_session_id,
            "lease_id": self.config.lease_id,
            "lease_epoch": self.config.lease_epoch,
            "generation": generation
        })
    }

    pub(super) fn call_tool_with_text(
        &mut self,
        name: &str,
        arguments: Value,
    ) -> Result<(Value, String), String> {
        self.call_tool_classified_with_text(name, arguments)
            .map_err(|error| error.message().to_owned())
    }

    /// Calls a gameplay MCP tool while retaining the exact JSON text carried by the MCP
    /// envelope. Durable checkpoints and operation intents bind to these bytes; serializing the
    /// parsed value again would lose the host's canonical wire representation.
    pub(super) fn call_tool_classified_with_text(
        &mut self,
        name: &str,
        arguments: Value,
    ) -> Result<(Value, String), RuntimeV3ToolError> {
        let id = self.next_rpc_id;
        self.next_rpc_id = self.next_rpc_id.checked_add(1).ok_or_else(|| {
            RuntimeV3ToolError::Terminal(String::from("MCP request identity exhausted"))
        })?;
        let request = json!({"name": name, "arguments": arguments});
        let recovery_read = matches!(name, "sts2.legal_actions" | "sts2.reobserve");
        let response = if name == "sts2.legal_actions" {
            wire::rpc_call_catalog_read(
                self.mcp_mut()
                    .map_err(|error| classify_mcp_error(error, recovery_read))?,
                id,
                "tools/call",
                request,
            )
        } else if name == "sts2.reobserve" {
            wire::rpc_call_recovery_read(
                self.mcp_mut()
                    .map_err(|error| classify_mcp_error(error, recovery_read))?,
                id,
                "tools/call",
                request,
            )
        } else {
            wire::rpc_call(
                self.mcp_mut()
                    .map_err(|error| classify_mcp_error(error, recovery_read))?,
                id,
                "tools/call",
                request,
            )
        }
        .map_err(|error| RuntimeV3ToolError::from_rpc_for(error, recovery_read))?;
        let text = response
            .get("result")
            .and_then(|result| result.get("content"))
            .and_then(Value::as_array)
            .and_then(|content| content.first())
            .and_then(|content| content.get("text"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                RuntimeV3ToolError::Terminal(format!("MCP tool {name} omitted text content"))
            })?;
        let value: Value = serde_json::from_str(text).map_err(|error| {
            RuntimeV3ToolError::Terminal(format!(
                "MCP tool {name} returned non-JSON content: {error}"
            ))
        })?;
        if wire::catalog_reobserve(&value)
            && (name != "sts2.legal_actions"
                || text.len() > 1024
                || response["result"]["isError"] != true)
        {
            return Err(RuntimeV3ToolError::Terminal(String::from(
                "MCP catalog recovery has an invalid tool envelope",
            )));
        }
        let expected_correlation = id.to_string();
        if value.get("correlation_id").and_then(Value::as_str)
            != Some(expected_correlation.as_str())
        {
            return Err(RuntimeV3ToolError::Terminal(format!(
                "MCP tool {name} returned mismatched correlation"
            )));
        }
        Ok((value, text.to_owned()))
    }

    fn mcp_mut(&mut self) -> Result<&mut McpProcess, sts2_harness::PortError> {
        self.mcp
            .as_mut()
            .ok_or_else(|| wire::port_error("mcp_unavailable", "MCP process is not running", false))
    }
}
