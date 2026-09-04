// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use serde_json::{Value, json};
use sts2_harness::{
    ActionIdentity, BarrierError, BarrierPort, EpisodeLegalAction, EpisodeLegalActionSet,
    EpisodeObservation, EpisodeRunner, EpisodeRuntimePort, ExoDecisionSource, ExoProcessTransport,
    ExoProvider, ExoSession, RecoveryError, RecoveryPort, ShutdownError, ShutdownPort,
    TransitionReceipt, WaitSample,
};

use super::config::RuntimeConfig;
use super::http::GatewayClient;
use super::mcp::{McpProcess, identity_headers, validate_allocation};
use super::runtime_v3_parse as parse;
use super::runtime_v3_settings::RuntimeV3Settings;
use super::runtime_v3_wire as wire;

#[path = "runtime_v3_ledger.rs"]
mod ledger;
use ledger::OperationRecord;

const MAX_OPERATIONS: usize = 1_024;

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

impl EpisodeRuntimePort for RuntimeV3Port {
    fn launch(&mut self) -> Result<(), sts2_harness::PortError> {
        if self.allocated {
            return Err(wire::port_error(
                "duplicate_launch",
                "episode is already allocated",
                false,
            ));
        }
        let allocation = self
            .gateway
            .request(
                "POST",
                "/v1/sessions/allocate",
                &json!({
                    "instance_id": self.config.instance_id,
                    "caller_id": self.config.caller_id,
                    "session_id": self.config.session_id
                }),
                BTreeMap::new(),
            )
            .map_err(|error| wire::port_error("gateway_allocate_failed", error, false))?;
        self.allocated = true;
        if let Err(error) = validate_allocation(&allocation, &self.config) {
            let release = self.release_lease_inner();
            return Err(wire::port_error(
                "gateway_allocate_invalid",
                wire::combine_cleanup(error, Ok(()), release),
                false,
            ));
        }
        if let Err(error) = self.launch_mcp() {
            return Err(wire::port_error("runtime_launch_failed", error, false));
        }
        Ok(())
    }

    fn observe(&mut self) -> Result<EpisodeObservation, sts2_harness::PortError> {
        let arguments = self.context(self.generation);
        let value = self
            .call_tool("sts2.observe", arguments)
            .map_err(|error| wire::port_error("observe_failed", error, false))?;
        let parsed = parse::observation(&value, "state_response", &self.config)
            .map_err(|error| wire::port_error("observe_invalid", error, false))?;
        Ok(self.install(parsed))
    }

    fn legal_actions(
        &mut self,
        state_id: &str,
        generation: u64,
    ) -> Result<EpisodeLegalActionSet, sts2_harness::PortError> {
        let mut arguments = self.context(generation);
        if let Value::Object(object) = &mut arguments {
            object.insert(String::from("state_id"), Value::String(state_id.to_owned()));
        }
        let value = self
            .call_tool("sts2.legal_actions", arguments)
            .map_err(|error| wire::port_error("legal_actions_failed", error, false))?;
        let (actions, payloads) = parse::action_set(&value, "legal_actions_response", &self.config)
            .map_err(|error| wire::port_error("legal_actions_invalid", error, false))?;
        self.generation = actions.generation();
        self.current_state = Some(actions.state_id().to_owned());
        self.current_actions = Some(actions.clone());
        self.payloads = payloads;
        Ok(actions)
    }

    fn dispatch_action(
        &mut self,
        identity: &ActionIdentity,
        action: &EpisodeLegalAction,
    ) -> Result<TransitionReceipt, sts2_harness::PortError> {
        if self.current_state.as_deref() != Some(identity.state_id.as_str())
            || self.generation != identity.generation
            || self
                .current_actions
                .as_ref()
                .and_then(|set| set.find(action.action_id()))
                != Some(action)
        {
            return Err(wire::port_error(
                "action_not_current",
                "dispatch action is not bound to the current host catalog",
                false,
            ));
        }
        let payload = self
            .payloads
            .get(action.action_id())
            .cloned()
            .ok_or_else(|| {
                wire::port_error(
                    "action_payload_missing",
                    "current legal action payload is unavailable",
                    false,
                )
            })?;
        if payload.get("kind").and_then(Value::as_str)
            != Some(wire::action_kind_name(action.kind()))
        {
            return Err(wire::port_error(
                "action_payload_mismatch",
                "legal action kind changed",
                false,
            ));
        }
        if let Some(existing) = self.operations.get(&identity.operation_id)
            && (existing.action != *action
                || existing.generation != identity.generation
                || existing.state_id != identity.state_id)
        {
            return Err(wire::port_error(
                "operation_conflict",
                "operation identity conflicts",
                false,
            ));
        }
        if self.operations.len() >= MAX_OPERATIONS
            && !self.operations.contains_key(&identity.operation_id)
        {
            return Err(wire::port_error(
                "operation_capacity",
                "operation ledger is full",
                false,
            ));
        }
        self.operations
            .entry(identity.operation_id.clone())
            .or_insert_with(|| OperationRecord::new(identity, action));
        let value = self
            .call_tool(
                "sts2.dispatch_action",
                json!({
                    "instance_id": self.config.instance_id,
                    "mcp_session_id": self.config.mcp_session_id,
                    "lease_id": self.config.lease_id,
                    "lease_epoch": self.config.lease_epoch,
                    "generation": identity.generation,
                    "state_id": identity.state_id,
                    "operation_id": identity.operation_id,
                    "action": payload
                }),
            )
            .map_err(|error| wire::port_error("dispatch_failed", error, true))?;
        let receipt = parse::receipt(
            &value,
            "dispatch_action_response",
            &self.config,
            &identity.operation_id,
            identity.generation,
            action.clone(),
        )
        .map_err(|error| wire::port_error("dispatch_invalid", error, false))?;
        self.install_response(&value, "dispatch_action_response")
            .map_err(|error| wire::port_error("dispatch_observation_invalid", error, false))?;
        Ok(receipt)
    }
}

impl BarrierPort for RuntimeV3Port {
    fn wait_for_transition(
        &mut self,
        operation_id: &str,
        wait_for_millis: u32,
    ) -> Result<WaitSample, BarrierError> {
        let value = self
            .call_tool(
                "sts2.wait_for_transition",
                json!({
                    "instance_id": self.config.instance_id,
                    "mcp_session_id": self.config.mcp_session_id,
                    "lease_id": self.config.lease_id,
                    "lease_epoch": self.config.lease_epoch,
                    "generation": self.generation,
                    "operation_id": operation_id,
                    "wait_for_millis": wait_for_millis
                }),
            )
            .map_err(|_| BarrierError::PortFailure)?;
        let expected_generation = self
            .operations
            .get(operation_id)
            .map_or(self.generation, |record| record.generation);
        let sample = parse::wait_sample(&value, &self.config, operation_id, expected_generation)
            .map_err(|_| BarrierError::PortFailure)?;
        self.install_response(&value, "wait_response")
            .map_err(|_| BarrierError::PortFailure)?;
        Ok(sample)
    }
}

impl RecoveryPort for RuntimeV3Port {
    fn reobserve(&mut self) -> Result<EpisodeObservation, RecoveryError> {
        let value = self
            .call_tool("sts2.reobserve", self.context(self.generation))
            .map_err(|_| RecoveryError::PortFailure)?;
        let parsed = parse::observation(&value, "reobserve_response", &self.config)
            .map_err(|_| RecoveryError::PortFailure)?;
        Ok(self.install(parsed))
    }

    fn reconcile(&mut self, operation_id: &str) -> Result<TransitionReceipt, RecoveryError> {
        let record = self
            .operations
            .get(operation_id)
            .cloned()
            .ok_or(RecoveryError::InvalidOperation)?;
        let value = self
            .call_tool(
                "sts2.recover",
                json!({
                    "instance_id": self.config.instance_id,
                    "mcp_session_id": self.config.mcp_session_id,
                    "lease_id": self.config.lease_id,
                    "lease_epoch": self.config.lease_epoch,
                    "generation": self.generation,
                    "recovery_kind": "reconcile",
                    "operation_id": operation_id
                }),
            )
            .map_err(|_| RecoveryError::PortFailure)?;
        let receipt = parse::receipt(
            &value,
            "recover_response",
            &self.config,
            operation_id,
            record.generation,
            record.action,
        )
        .map_err(|_| RecoveryError::PortFailure)?;
        self.install_response(&value, "recover_response")
            .map_err(|_| RecoveryError::PortFailure)?;
        Ok(receipt)
    }

    fn release_lease(&mut self) -> Result<(), RecoveryError> {
        self.release_lease_inner()
            .map_err(|_| RecoveryError::PortFailure)
    }

    fn stop_episode(&mut self) -> Result<(), RecoveryError> {
        RecoveryPort::release_lease(self)
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
