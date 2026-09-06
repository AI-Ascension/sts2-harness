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
use super::runtime_v3_telemetry::{
    CleanupStatus, GameOutcome, RuntimeV3Telemetry, TelemetryContext, TelemetryHandle,
    TelemetryStage,
};
use super::runtime_v3_wire as wire;

#[path = "runtime_v3_episode.rs"]
mod episode;
#[path = "runtime_v3_ledger.rs"]
mod ledger;
#[path = "runtime_v3_recording.rs"]
mod recording;
#[path = "runtime_v3_recovery.rs"]
mod recovery;
#[path = "runtime_v3_wait.rs"]
mod wait;
use ledger::OperationRecord;

#[path = "runtime_v3_combat_demo.rs"]
mod combat_demo;
#[path = "runtime_v3_episode_replay.rs"]
mod episode_replay;

#[cfg(test)]
#[path = "runtime_v3_lifecycle_test.rs"]
mod lifecycle_tests;

pub(super) fn run(config: RuntimeConfig) -> Result<(), String> {
    let settings = RuntimeV3Settings::from_environment()?;
    let telemetry_context = TelemetryContext::new(
        &config.run_id,
        &config.episode_id,
        &config.trajectory_id,
        &config.trace_id,
        &config.instance_id,
        &config.session_id,
        &config.runtime_profile,
        &settings.exo.revision,
    )?;
    let telemetry = RuntimeV3Telemetry::new(telemetry_context);
    let telemetry_handle = telemetry.handle();
    let _ = telemetry_handle.run_started();
    let mut port = match RuntimeV3Port::new_with_telemetry(config, telemetry_handle.clone()) {
        Ok(port) => port,
        Err(error) => {
            let _ = telemetry_handle.failure(
                "runtime_init",
                super::runtime_v3_telemetry::FailureCode::Configuration,
                false,
                None,
            );
            let _ = telemetry_handle.run_finished(
                GameOutcome::Unavailable,
                TelemetryStage::Unknown,
                CleanupStatus::Failed,
            );
            finish_telemetry(telemetry);
            return Err(error);
        }
    };
    if std::env::var("STS2_COMBAT_DEMO").as_deref() != Ok("true") {
        let path = std::env::var("STS2_REPLAY_TRAJECTORY").unwrap_or_default();
        if !path.is_empty() {
            let result = episode_replay::run(&mut port, &settings.runner, &path);
            drop(port);
            let _ = telemetry_handle.run_finished(
                if result.is_ok() {
                    GameOutcome::Success
                } else {
                    GameOutcome::Failure
                },
                TelemetryStage::Unknown,
                CleanupStatus::Clean,
            );
            finish_telemetry(telemetry);
            return result;
        }
    }
    let transport = ExoProcessTransport::new(settings.process);
    let provider = ExoProvider::new(transport, settings.exo);
    let mut source = ExoDecisionSource::new(ExoSession::new(provider));
    if std::env::var("STS2_COMBAT_DEMO").as_deref() == Ok("true") {
        let outcome = combat_demo::run(&mut port, &mut source, &settings.runner);
        let close = source.close().map_err(|error| error.to_string());
        drop(port);
        let game_outcome = if outcome.is_ok() && close.is_ok() {
            GameOutcome::Success
        } else {
            GameOutcome::Failure
        };
        let _ = telemetry_handle.run_finished(
            game_outcome,
            TelemetryStage::Unknown,
            if close.is_ok() {
                CleanupStatus::Clean
            } else {
                CleanupStatus::Failed
            },
        );
        finish_telemetry(telemetry);
        return outcome.and(close);
    }
    let result = EpisodeRunner::new(settings.runner).run(
        &mut port,
        &mut recording::DecisionRecorder::new(&mut source, telemetry_handle.clone()),
    );
    let source_close = source.close();
    drop(port);
    let report = match result {
        Ok(report) => report,
        Err(error) => {
            let _ = telemetry_handle.failure(
                "episode",
                super::runtime_v3_telemetry::FailureCode::Other,
                false,
                None,
            );
            let _ = telemetry_handle.run_finished(
                GameOutcome::Unavailable,
                TelemetryStage::Unknown,
                CleanupStatus::Failed,
            );
            finish_telemetry(telemetry);
            return Err(format!("Runtime-v3 episode failed: {error}"));
        }
    };
    if source_close.is_err() {
        let _ = telemetry_handle.failure(
            "provider_close",
            super::runtime_v3_telemetry::FailureCode::Cleanup,
            false,
            None,
        );
        let _ = telemetry_handle.run_finished(
            GameOutcome::Unavailable,
            TelemetryStage::from(report.terminal_stage()),
            CleanupStatus::Failed,
        );
        finish_telemetry(telemetry);
        return Err(String::from("Exo session close failed"));
    }
    recording::complete(&report, &telemetry_handle);
    let game_outcome = match report.terminal_stage() {
        sts2_harness::EpisodeStage::Victory => GameOutcome::Success,
        sts2_harness::EpisodeStage::Defeat => GameOutcome::Failure,
        _ => GameOutcome::Unavailable,
    };
    let _ = telemetry_handle.run_finished(
        game_outcome,
        TelemetryStage::from(report.terminal_stage()),
        CleanupStatus::Clean,
    );
    finish_telemetry(telemetry);
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
    reconnect_attempts: u8,
    telemetry: TelemetryHandle,
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
            allocated: false,
            released: false,
            next_rpc_id: 1,
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
