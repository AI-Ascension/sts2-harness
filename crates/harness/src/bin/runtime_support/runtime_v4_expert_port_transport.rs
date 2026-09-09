// SPDX-License-Identifier: MIT

include!("runtime_v4_expert_port_profile.rs");

impl RuntimeV3Port {
    fn expert_mcp_mut(&mut self) -> Result<&mut super::super::mcp::McpProcess, String> {
        self.expert_mcp
            .as_mut()
            .ok_or_else(|| String::from("expert MCP process is not running"))
    }

    fn call_expert_tool_classified(
        &mut self,
        name: &str,
        arguments: Value,
    ) -> Result<(u64, Value), RuntimeV3ToolError> {
        self.call_expert_tool_classified_mode(name, arguments, false)
    }

    fn call_expert_tool_classified_catalog(
        &mut self,
        name: &str,
        arguments: Value,
    ) -> Result<(u64, Value), RuntimeV3ToolError> {
        self.call_expert_tool_classified_mode(name, arguments, true)
    }

    fn call_expert_tool_classified_mode(
        &mut self,
        name: &str,
        arguments: Value,
        catalog_read: bool,
    ) -> Result<(u64, Value), RuntimeV3ToolError> {
        let id = self.expert_next_rpc_id;
        self.expert_next_rpc_id = self
            .expert_next_rpc_id
            .checked_add(1)
            .ok_or_else(|| {
                RuntimeV3ToolError::Terminal(String::from(
                    "expert MCP request identity exhausted",
                ))
            })?;
        let request = json!({"name": name, "arguments": arguments});
        let response = if catalog_read {
            wire::rpc_call_catalog_read(
                self.expert_mcp_mut()
                    .map_err(|error| RuntimeV3ToolError::Transient(error.to_string()))?,
                id,
                "tools/call",
                request,
            )
        } else {
            wire::rpc_call(
                self.expert_mcp_mut()
                    .map_err(|error| {
                        if catalog_read {
                            RuntimeV3ToolError::Transient(error.to_string())
                        } else {
                            RuntimeV3ToolError::Terminal(error.to_string())
                        }
                    })?,
                id,
                "tools/call",
                request,
            )
        }
        .map_err(|error| RuntimeV3ToolError::from_rpc_for(error, catalog_read))?;
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
        let value: Value = serde_json::from_str(text)
            .map_err(|error| {
                RuntimeV3ToolError::Terminal(format!(
                    "MCP tool {name} returned non-JSON content: {error}"
                ))
            })?;
        if name != STATE_TOOL
            && value.get("correlation_id").and_then(Value::as_str) != Some(id.to_string().as_str())
        {
            return Err(RuntimeV3ToolError::Terminal(format!(
                "MCP tool {name} returned mismatched correlation"
            )));
        }
        Ok((id, value))
    }

    pub(super) fn call_expert_tool(
        &mut self,
        name: &str,
        arguments: Value,
    ) -> Result<(u64, Value), String> {
        self.call_expert_tool_classified(name, arguments)
            .map_err(|error| error.message().to_owned())
    }

    fn expert_state_classified(
        &mut self,
        catalog_read: bool,
    ) -> Result<RuntimeV4ExpertObservation, RuntimeV3ToolError> {
        let (_, value) = self.call_expert_tool_classified_mode(
            STATE_TOOL,
            json!({
                "instance_id": self.config.instance_id,
                "mcp_session_id": self.config.mcp_session_id
            }),
            catalog_read,
        )?;
        RuntimeV4ExpertObservation::from_value(value).map_err(|error| {
            RuntimeV3ToolError::Terminal(format!("Runtime-v4 expert state is invalid: {error}"))
        })
    }

    pub(super) fn merge_current_expert_actions(
        &mut self,
        state_id: &str,
        generation: u64,
    ) -> Result<(), RuntimeV3ToolError> {
        let (_, value) = self.call_expert_tool_classified_catalog(
            STATE_TOOL,
            json!({
                "instance_id": self.config.instance_id,
                "mcp_session_id": self.config.mcp_session_id
            }),
        )?;
        let expert = RuntimeV4ExpertObservation::from_value(value).map_err(|error| {
            RuntimeV3ToolError::Terminal(format!("Runtime-v4 expert state is invalid: {error}"))
        })?;
        if expert.state_id() != state_id || expert.generation() != generation {
            // A newer expert snapshot means the normal catalog became stale during the two
            // projection reads. Let the existing catalog-reobserve recovery path obtain a new
            // pair; equal-generation identity changes and retrograde data remain terminal.
            if expert.generation() > generation {
                return Err(RuntimeV3ToolError::Transient(String::from(
                    "Runtime-v4 expert catalog advanced beyond the Runtime-v3 observation",
                )));
            }
            return Err(RuntimeV3ToolError::Terminal(String::from(
                "Runtime-v4 expert catalog does not match the Runtime-v3 observation",
            )));
        }
        let normal_actions = self
            .current_actions
            .clone()
            .ok_or_else(|| {
                RuntimeV3ToolError::Terminal(String::from(
                    "normal catalog is unavailable for expert merge",
                ))
            })?;
        let normal_payloads = self.payloads.clone();
        let (actions, payloads) = merge_actions(&normal_actions, &normal_payloads, &expert)
            .map_err(RuntimeV3ToolError::Terminal)?;
        self.current_actions = Some(actions);
        self.payloads = payloads;
        Ok(())
    }

    pub(super) fn dispatch_expert_action(
        &mut self,
        identity: &ActionIdentity,
        action: &EpisodeLegalAction,
        payload: Value,
    ) -> Result<TransitionReceipt, sts2_harness::PortError> {
        if self.is_rest_profile() {
            return self.dispatch_rest_action(identity, action, payload);
        }
        let request_value = action_request(&self.config, identity, action, &payload, "request");
        let request = RuntimeV4ExpertActionRequest::from_value(request_value).map_err(|error| {
            wire::port_error("expert_request_invalid", error.to_string(), false)
        })?;
        let (_, value) = self
            .call_expert_tool(
                ACTION_TOOL,
                json!({
                    "instance_id": self.config.instance_id,
                    "mcp_session_id": self.config.mcp_session_id,
                    "lease_id": self.config.lease_id,
                    "lease_epoch": self.config.lease_epoch,
                    "generation": identity.generation,
                    "state_id": identity.state_id,
                    "operation_id": identity.operation_id,
                    "action": {"action_id": action.action_id(), "action": payload}
                }),
            )
            .map_err(|error| wire::port_error("expert_dispatch_failed", error, true))?;
        let result = RuntimeV4ExpertActionResult::from_value(value).map_err(|error| {
            wire::port_error("expert_dispatch_invalid", error.to_string(), false)
        })?;
        validate_expert_result(&result, &request, identity, action)
            .map_err(|error| wire::port_error("expert_dispatch_invalid", error, false))?;
        let receipt = self
            .expert_result_receipt(result, &request, identity, action)
            .map_err(|error| wire::port_error("expert_receipt_invalid", error, false))?;
        super::recording::receipt(&receipt, identity.generation, &self.telemetry);
        Ok(receipt)
    }

    pub(super) fn reconcile_expert_operation(
        &mut self,
        operation_id: &str,
    ) -> Result<TransitionReceipt, String> {
        let record = self
            .operations
            .get(operation_id)
            .cloned()
            .ok_or_else(|| String::from("expert operation is not in the ledger"))?;
        if !self.uses_expert_transport(&record.action, &record.payload) {
            return Err(String::from("operation is not a Runtime-v4 expert action"));
        }
        if self.is_rest_profile() {
            return self.reconcile_rest_operation(operation_id);
        }
        let identity = ActionIdentity::new(
            operation_id.to_owned(),
            record.state_id.clone(),
            record.generation,
            record.action.action_id().to_owned(),
        )
        .map_err(|error| error.to_string())?;
        let request = RuntimeV4ExpertActionRequest::from_value(action_request(
            &self.config,
            &identity,
            &record.action,
            &record.payload,
            "request",
        ))
        .map_err(|error| error.to_string())?;
        let (_, value) = self.call_expert_tool(
            RECONCILE_TOOL,
            json!({
                "instance_id": self.config.instance_id,
                "mcp_session_id": self.config.mcp_session_id,
                "lease_id": self.config.lease_id,
                "lease_epoch": self.config.lease_epoch,
                "operation_id": operation_id
            }),
        )?;
        let result = RuntimeV4ExpertActionResult::from_value(value)
            .map_err(|error| format!("Runtime-v4 expert reconcile response is invalid: {error}"))?;
        validate_expert_result(&result, &request, &identity, &record.action)?;
        self.expert_result_receipt(result, &request, &identity, &record.action)
    }

    pub(super) fn wait_expert_operation(
        &mut self,
        operation_id: &str,
    ) -> Result<WaitSample, String> {
        let receipt = self.reconcile_expert_operation(operation_id)?;
        match receipt.status() {
            DispatchStatus::Settled => {
                Ok(
                    WaitSample::new(WaitOutcome::SameStateMutation, receipt.after().cloned())
                        .with_effect_kind(receipt.effect_kind().ok_or_else(|| {
                            String::from("expert settlement omitted effect witness")
                        })?),
                )
            }
            DispatchStatus::Accepted | DispatchStatus::Unknown => {
                Ok(WaitSample::new(WaitOutcome::Timeout, None))
            }
            DispatchStatus::Rejected | DispatchStatus::Cancelled => {
                Ok(WaitSample::new(WaitOutcome::RecoveryRequired, None))
            }
        }
    }

}

include!("runtime_v4_expert_port_transport_receipt.rs");
include!("runtime_v4_expert_port_transport_composition.rs");
include!("runtime_v4_expert_port_transport_observation.rs");
