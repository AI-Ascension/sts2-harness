// SPDX-License-Identifier: MIT

#[derive(Debug)]
pub(super) enum RuntimeV3ToolError {
    Transient(String),
    Terminal(String),
}

impl RuntimeV3ToolError {
    fn from_rpc_for(error: wire::RpcFailure, transient_allowed: bool) -> Self {
        if transient_allowed && error.is_transient() {
            Self::Transient(error.to_string())
        } else {
            Self::Terminal(error.to_string())
        }
    }

    pub(super) fn message(&self) -> &str {
        match self {
            Self::Transient(message) | Self::Terminal(message) => message,
        }
    }
}

fn classify_mcp_error(
    error: sts2_harness::PortError,
    transient_allowed: bool,
) -> RuntimeV3ToolError {
    if transient_allowed {
        RuntimeV3ToolError::Transient(error.to_string())
    } else {
        RuntimeV3ToolError::Terminal(error.to_string())
    }
}

fn finish_telemetry(telemetry: RuntimeV3Telemetry) {
    let report = telemetry.finish(std::time::Duration::from_secs(2));
    if report.export_status() != "delivered" {
        eprintln!(
            "runtime-v3 telemetry export status={} sent={} failed={} dropped={} timed_out={}",
            report.export_status(),
            report.sent,
            report.failed,
            report
                .normal_dropped
                .saturating_add(report.critical_dropped),
            report.timed_out
        );
    }
}

impl RuntimeV3Port {
    fn new_with_telemetry(
        config: RuntimeConfig,
        telemetry: TelemetryHandle,
    ) -> Result<Self, String> {
        let gateway = GatewayClient::new(&config)?;
        Ok(Self {
            config,
            gateway,
            mcp: None,
            expert_mcp: None,
            allocated: false,
            released: false,
            next_rpc_id: 1,
            expert_next_rpc_id: 1,
            generation: 0,
            current_state: None,
            current_actions: None,
            payloads: BTreeMap::new(),
            operations: BTreeMap::new(),
            reconnect_attempts: 0,
            telemetry,
        })
    }

    fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value, String> {
        self.call_tool_classified(name, arguments)
            .map_err(|error| error.message().to_owned())
    }

    pub(super) fn call_tool_classified(
        &mut self,
        name: &str,
        arguments: Value,
    ) -> Result<Value, RuntimeV3ToolError> {
        let id = self.next_rpc_id;
        self.next_rpc_id = self
            .next_rpc_id
            .checked_add(1)
            .ok_or_else(|| {
                RuntimeV3ToolError::Terminal(String::from("MCP request identity exhausted"))
        })?;
        let request = json!({"name": name, "arguments": arguments});
        let recovery_read =
            matches!(name, "sts2.legal_actions" | "sts2.reobserve") || name == "sts2.coop_receipt_query";
        let response = if name == "sts2.legal_actions" {
            wire::rpc_call_catalog_read(
                self.mcp_mut()
                    .map_err(|error| classify_mcp_error(error, recovery_read))?,
                id,
                "tools/call",
                request,
            )
        } else if recovery_read {
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
        let value: Value = serde_json::from_str(text)
            .map_err(|error| {
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
        Ok(value)
    }

    fn mcp_mut(&mut self) -> Result<&mut McpProcess, sts2_harness::PortError> {
        self.mcp
            .as_mut()
            .ok_or_else(|| wire::port_error("mcp_unavailable", "MCP process is not running", false))
    }

    fn context(&self, generation: u64) -> Value {
        json!({
            "instance_id": self.config.instance_id,
            "mcp_session_id": self.config.mcp_session_id,
            "lease_id": self.config.lease_id,
            "lease_epoch": self.config.lease_epoch,
            "generation": generation
        })
    }

    fn expert_catalog(
        &mut self,
        state_id: &str,
        generation: u64,
    ) -> Result<EpisodeLegalActionSet, sts2_harness::PortError> {
        match self.merge_current_expert_actions(state_id, generation) {
            Ok(()) => self.current_actions.clone().ok_or_else(|| {
                wire::port_error(
                    "expert_legal_actions_invalid",
                    "expert catalog was not installed",
                    false,
                )
            }),
            Err(RuntimeV3ToolError::Transient(error)) => Err(wire::port_error(
                "catalog_reobserve",
                format!("expert catalog transport failed: {error}"),
                true,
            )),
            Err(RuntimeV3ToolError::Terminal(error)) => Err(wire::port_error(
                "expert_legal_actions_invalid",
                error,
                false,
            )),
        }
    }

    fn install(&mut self, parsed: parse::ParsedObservation) -> EpisodeObservation {
        self.generation = parsed.observation.generation();
        self.current_state = Some(parsed.observation.state_id().to_owned());
        self.current_actions = Some(parsed.actions);
        self.payloads = parsed.payloads;
        parsed.observation
    }

    fn install_response(&mut self, value: &Value, expected_kind: &str) -> Result<(), String> {
        if value
            .get("observation")
            .is_some_and(|observation| observation.is_object())
        {
            let parsed = parse::result_observation(value, expected_kind, &self.config)?;
            let _ = self.install(parsed);
        }
        Ok(())
    }

    fn release_lease_inner(&mut self) -> Result<(), String> {
        if !self.allocated || self.released {
            return Ok(());
        }
        let response = self.gateway.request(
            "POST",
            &format!("/v1/instances/{}/release", self.config.instance_id),
            &Value::Null,
            identity_headers(&self.config, "release-0001"),
        )?;
        if response.get("status").and_then(Value::as_str) != Some("released") {
            return Err(String::from(
                "gateway release did not return released status",
            ));
        }
        self.released = true;
        Ok(())
    }

    fn launch_mcp(&mut self) -> Result<(), String> {
        let normal_profile = if self.is_expert_profile() {
            "runtime-v3-gameplay"
        } else {
            self.config.runtime_profile.as_str()
        };
        let mut mcp = match McpProcess::spawn_profile(&self.config, normal_profile) {
            Ok(mcp) => mcp,
            Err(error) => {
                let release = self.release_lease_inner();
                return Err(wire::combine_cleanup(error, Ok(()), release));
            }
        };
        if let Err(error) = wire::initialize_mcp_profile(&mut mcp, normal_profile) {
            let close = mcp.close();
            let release = self.release_lease_inner();
            return Err(wire::combine_cleanup(error, close, release));
        }
        self.mcp = Some(mcp);
        if self.is_expert_profile() {
            let mut expert = match McpProcess::spawn_profile(&self.config, "runtime-v4-expert") {
                Ok(expert) => expert,
                Err(error) => {
                    let close = self
                        .mcp
                        .as_mut()
                        .map_or(Ok(()), McpProcess::close);
                    let release = self.release_lease_inner();
                    return Err(wire::combine_cleanup(error, close, release));
                }
            };
            if let Err(error) = wire::initialize_mcp_profile(&mut expert, "runtime-v4-expert") {
                let expert_close = expert.close();
                let normal_close = self
                    .mcp
                    .as_mut()
                    .map_or(Ok(()), McpProcess::close);
                let close = match (expert_close, normal_close) {
                    (Ok(()), Ok(())) => Ok(()),
                    (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
                    (Err(first), Err(second)) => Err(format!("{first}; {second}")),
                };
                let release = self.release_lease_inner();
                return Err(wire::combine_cleanup(error, close, release));
            }
            self.expert_mcp = Some(expert);
        }
        Ok(())
    }
}

impl ShutdownPort for RuntimeV3Port {
    fn release_lease(&mut self) -> Result<(), ShutdownError> {
        self.release_lease_inner()
            .map_err(|_| ShutdownError::ReleaseFailed)
    }

    fn close_mcp(&mut self) -> Result<(), ShutdownError> {
        let mut failure = None;
        if let Some(mcp) = self.expert_mcp.as_mut()
            && mcp.close().is_err()
        {
            failure = Some(ShutdownError::McpCloseFailed);
        }
        if let Some(mcp) = self.mcp.as_mut()
            && mcp.close().is_err()
            && failure.is_none()
        {
            failure = Some(ShutdownError::McpCloseFailed);
        }
        failure.map_or(Ok(()), Err)
    }

    fn close_gateway(&mut self) -> Result<(), ShutdownError> {
        if self.allocated && !self.released {
            return Err(ShutdownError::GatewayCloseFailed);
        }
        Ok(())
    }
}
