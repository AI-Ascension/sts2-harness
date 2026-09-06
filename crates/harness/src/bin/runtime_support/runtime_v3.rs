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

include!("runtime_v3_port.rs");
