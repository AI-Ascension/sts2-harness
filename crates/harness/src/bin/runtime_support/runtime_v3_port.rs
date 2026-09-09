// SPDX-License-Identifier: MIT

#[derive(Debug)]
pub(super) enum RuntimeV3ToolError {
    Transient(String),
    Terminal(String),
}

#[path = "runtime_v3_port_transport.rs"]
mod transport;

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
            catalog: None,
            catalog_raw: None,
            payloads: BTreeMap::new(),
            operations: BTreeMap::new(),
            reconnect_attempts: 0,
            telemetry,
            durable: None,
            recovery_authority: None,
            recovery: None,
            recovery_context: None,
            recovery_rpc_id: 1,
            cancellation: sts2_harness::ExecutionCancellation::default(),
        })
    }

    fn new_with_store(
        config: RuntimeConfig,
        telemetry: TelemetryHandle,
        durable: durable::DurableHandle,
    ) -> Result<Self, String> {
        let mut port = Self::new_with_telemetry(config, telemetry)?;
        port.durable = Some(durable);
        Ok(port)
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

    #[cfg(test)]
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
        self.durable
            .as_ref()
            .map_or(Ok(()), |durable| durable.mark_interrupted_unknown(reason))
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
            identity_headers(&self.config, &super::mcp::release_correlation()),
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
        let mut mcp = match if self.is_expert_profile() {
            McpProcess::spawn_profile(&self.config, normal_profile)
        } else {
            McpProcess::spawn_with_cancellation(&self.config, &self.cancellation)
        } {
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
                    let close = self.mcp.as_mut().map_or(Ok(()), McpProcess::close);
                    let release = self.release_lease_inner();
                    return Err(wire::combine_cleanup(error, close, release));
                }
            };
            if let Err(error) = wire::initialize_mcp_profile(&mut expert, "runtime-v4-expert") {
                let expert_close = expert.close();
                let normal_close = self.mcp.as_mut().map_or(Ok(()), McpProcess::close);
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
        if let Some(mcp) = self.recovery.as_mut()
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
