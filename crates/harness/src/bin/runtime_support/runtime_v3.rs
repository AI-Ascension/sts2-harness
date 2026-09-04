// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use serde_json::{Value, json};
use sts2_harness::{
    EpisodeLegalActionSet, EpisodeObservation, EpisodeRunner, ExoDecisionSource,
    ExoProcessTransport, ExoProvider, ExoSession, ShutdownError, ShutdownPort,
};

use super::config::RuntimeConfig;
use super::http::GatewayClient;
use super::mcp::{McpProcess, identity_headers};
use super::runtime_v3_parse as parse;
use super::runtime_v3_settings::RuntimeV3Settings;
use super::runtime_v3_wire as wire;

#[path = "runtime_v3_episode.rs"]
mod episode;
#[path = "runtime_v3_ledger.rs"]
mod ledger;
#[path = "runtime_v3_recovery.rs"]
mod recovery;
use ledger::OperationRecord;

pub(super) fn run(config: RuntimeConfig) -> Result<(), String> {
    let settings = RuntimeV3Settings::from_environment()?;
    let mut port = RuntimeV3Port::new(config)?;
    let transport = ExoProcessTransport::new(settings.process);
    let provider = ExoProvider::new(transport, settings.exo);
    let mut source = ExoDecisionSource::new(ExoSession::new(provider));
    let result = EpisodeRunner::new(settings.runner).run(&mut port, &mut source);
    let source_close = source.close();
    let report = result.map_err(|error| format!("Runtime-v3 episode failed: {error}"))?;
    source_close.map_err(|error| format!("Exo session close failed: {error}"))?;
    println!(
        "{}",
        serde_json::to_string(&json!({
            "protocol": "runtime-v3-gameplay",
            "status": "complete",
            "terminal_stage": wire::stage_name(report.terminal_stage()),
            "steps": report.steps(),
            "transitions": report.transitions(),
            "recoveries": report.recoveries(),
            "final_state_id": report.final_observation().state_id(),
            "final_generation": report.final_observation().generation()
        }))
        .map_err(|error| format!("Runtime-v3 report serialization failed: {error}"))?
    );
    Ok(())
}

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
    payloads: BTreeMap<String, Value>,
    operations: BTreeMap<String, OperationRecord>,
}

impl RuntimeV3Port {
    fn new(config: RuntimeConfig) -> Result<Self, String> {
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
            payloads: BTreeMap::new(),
            operations: BTreeMap::new(),
        })
    }

    fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value, String> {
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
        let expected_correlation = id.to_string();
        if value.get("correlation_id").and_then(Value::as_str)
            != Some(expected_correlation.as_str())
        {
            return Err(format!("MCP tool {name} returned mismatched correlation"));
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
        let mut mcp = match McpProcess::spawn(&self.config) {
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
        self.mcp.as_mut().map_or(Ok(()), |mcp| {
            mcp.close().map_err(|_| ShutdownError::McpCloseFailed)
        })
    }

    fn close_gateway(&mut self) -> Result<(), ShutdownError> {
        if self.allocated && !self.released {
            return Err(ShutdownError::GatewayCloseFailed);
        }
        Ok(())
    }
}
