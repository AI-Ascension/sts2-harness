// SPDX-License-Identifier: MIT

pub(super) struct RuntimeV3Port {
    config: RuntimeConfig,
    gateway: GatewayClient,
    mcp: Option<McpProcess>,
    allocated: bool,
    released: bool,
    next_rpc_id: u64,
    generation: u64,
    current_state: Option<String>,
    current_actions: Option<EpisodeLegalActionSet>,
    catalog: Option<Value>,
    catalog_raw: Option<Vec<u8>>,
    payloads: BTreeMap<String, Value>,
    operations: BTreeMap<String, OperationRecord>,
    reconnect_attempts: u8,
    telemetry: TelemetryHandle,
    durable: Option<durable::DurableHandle>,
    recovery_authority: Option<allocation_context::RecoveryAuthority>,
    recovery: Option<McpProcess>,
    recovery_context: Option<recovery::RecoveryContext>,
    recovery_rpc_id: u64,
    cancellation: sts2_harness::ExecutionCancellation,
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
    #[cfg(test)]
    fn new_with_telemetry(
        config: RuntimeConfig,
        telemetry: TelemetryHandle,
    ) -> Result<Self, String> {
        Self::new(config, telemetry, None)
    }

    fn new_with_store(
        config: RuntimeConfig,
        telemetry: TelemetryHandle,
        durable: durable::DurableHandle,
    ) -> Result<Self, String> {
        Self::new(config, telemetry, Some(durable))
    }

    fn new(
        config: RuntimeConfig,
        telemetry: TelemetryHandle,
        durable: Option<durable::DurableHandle>,
    ) -> Result<Self, String> {
        let gateway = GatewayClient::new(&config)?;
        Ok(Self {
            config,
            gateway,
            mcp: None,
            allocated: false,
            released: false,
            next_rpc_id: 1,
            generation: 0,
            current_state: None,
            current_actions: None,
            catalog: None,
            catalog_raw: None,
            payloads: BTreeMap::new(),
            operations: BTreeMap::new(),
            reconnect_attempts: 0,
            telemetry,
            durable,
            recovery_authority: None,
            recovery: None,
            recovery_context: None,
            recovery_rpc_id: 1,
            cancellation: sts2_harness::ExecutionCancellation::default(),
        })
    }

    fn durable_handle(&self) -> Option<durable::DurableHandle> {
        self.durable.clone()
    }

    pub(super) fn complete_durable(
        &self,
        report: &sts2_harness::EpisodeRunReport,
    ) -> Result<(), String> {
        self.durable
            .as_ref()
            .map_or(Ok(()), |durable| durable.complete_episode(report))
    }

    pub(super) fn complete_durable_observation(
        &self,
        observation: &EpisodeObservation,
    ) -> Result<(), String> {
        self.durable
            .as_ref()
            .map_or(Ok(()), |durable| durable.complete_observation(observation))
    }

    pub(super) fn close_durable(&self) -> Result<(), String> {
        self.durable
            .as_ref()
            .map_or(Ok(()), durable::DurableHandle::close)
    }

    pub(super) fn mark_interrupted_unknown(&self, reason: &str) -> Result<(), String> {
        if let Some(durable) = &self.durable {
            durable.mark_interrupted_unknown(reason)
        } else {
            Ok(())
        }
    }

    fn call_tool(&mut self, name: &str, arguments: Value) -> Result<(Value, String), String> {
        let id = self.next_rpc_id;
        self.next_rpc_id = self
            .next_rpc_id
            .checked_add(1)
            .ok_or_else(|| String::from("MCP request identity exhausted"))?;
        let response = wire::rpc_call(
            self.mcp_mut().map_err(|error| error.to_string())?,
            id,
            "tools/call",
            json!({"name": name, "arguments": arguments}),
        )?;
        let text = response
            .get("result")
            .and_then(|result| result.get("content"))
            .and_then(Value::as_array)
            .and_then(|content| content.first())
            .and_then(|content| content.get("text"))
            .and_then(Value::as_str)
            .ok_or_else(|| format!("MCP tool {name} omitted text content"))?;
        let value: Value = serde_json::from_str(text)
            .map_err(|error| format!("MCP tool {name} returned non-JSON content: {error}"))?;
        if wire::catalog_reobserve(&value)
            && (name != "sts2.legal_actions"
                || text.len() > 1024
                || response["result"]["isError"] != true)
        {
            return Err(String::from(
                "MCP catalog recovery has an invalid tool envelope",
            ));
        }
        let expected_correlation = id.to_string();
        if value.get("correlation_id").and_then(Value::as_str)
            != Some(expected_correlation.as_str())
        {
            return Err(format!("MCP tool {name} returned mismatched correlation"));
        }
        Ok((value, text.to_owned()))
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

    fn install(&mut self, parsed: parse::ParsedObservation) -> Result<EpisodeObservation, String> {
        self.generation = parsed.observation.generation();
        self.current_state = Some(parsed.observation.state_id().to_owned());
        self.current_actions = Some(parsed.actions);
        self.catalog = Some(parsed.catalog);
        self.catalog_raw = Some(parsed.catalog_raw.clone());
        self.payloads = parsed.payloads;
        if let Some(durable) = &self.durable {
            durable.checkpoint_raw(&parsed.observation, &parsed.catalog_raw)?;
        }
        Ok(parsed.observation)
    }

    fn install_response(
        &mut self,
        value: &Value,
        response_text: &str,
        expected_kind: &str,
    ) -> Result<(), String> {
        if value
            .get("observation")
            .is_some_and(|observation| observation.is_object())
        {
            let parsed = parse::result_observation_with_text(
                value,
                response_text,
                expected_kind,
                &self.config,
            )?;
            let _ = self.install(parsed)?;
        }
        Ok(())
    }

    fn release_lease_inner(&mut self) -> Result<(), String> {
        if !self.allocated || self.released {
            return Ok(());
        }
        let response = self.gateway.release(
            &self.config.instance_id,
            &Value::Null,
            identity_headers(&self.config, &release_correlation()),
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
        let mut mcp = match McpProcess::spawn_with_cancellation(&self.config, &self.cancellation) {
            Ok(mcp) => mcp,
            Err(error) => {
                let release = self.release_lease_inner();
                return Err(wire::combine_cleanup(error, Ok(()), release));
            }
        };
        if let Err(error) = wire::initialize_mcp(&mut mcp) {
            let close = mcp.close();
            let release = self.release_lease_inner();
            return Err(wire::combine_cleanup(error, close, release));
        }
        self.mcp = Some(mcp);
        Ok(())
    }
}

impl ShutdownPort for RuntimeV3Port {
    fn release_lease(&mut self) -> Result<(), ShutdownError> {
        self.release_lease_inner()
            .map_err(|_| ShutdownError::ReleaseFailed)
    }

    fn close_mcp(&mut self) -> Result<(), ShutdownError> {
        let mut failed = false;
        if let Some(mcp) = self.mcp.as_mut()
            && mcp.close().is_err()
        {
            failed = true;
        }
        if let Some(mcp) = self.recovery.as_mut()
            && mcp.close().is_err()
        {
            failed = true;
        }
        if failed {
            Err(ShutdownError::McpCloseFailed)
        } else {
            Ok(())
        }
    }

    fn close_gateway(&mut self) -> Result<(), ShutdownError> {
        if self.allocated && !self.released {
            return Err(ShutdownError::GatewayCloseFailed);
        }
        Ok(())
    }
}
