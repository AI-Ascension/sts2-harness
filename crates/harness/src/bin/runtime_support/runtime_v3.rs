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
    CleanupStatus, RuntimeV3Telemetry, TelemetryContext, TelemetryContextInput, TelemetryHandle,
    TelemetryStage,
};
use super::runtime_v3_wire as wire;

#[path = "runtime_v3_episode.rs"]
mod episode;
#[path = "runtime_v4_expert_port.rs"]
mod expert;
#[path = "runtime_v3_ledger.rs"]
mod ledger;
#[path = "runtime_v3_receipt_query.rs"]
mod receipt_query;
#[path = "runtime_v3_recording.rs"]
mod recording;
#[path = "runtime_v3_recovery.rs"]
mod recovery;
#[path = "runtime_map.rs"]
mod runtime_map;
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
    let runtime_profile = config.runtime_profile.clone();
    let settings = RuntimeV3Settings::from_environment(&config)?;
    let telemetry_context = TelemetryContext::new(TelemetryContextInput {
        run_id: &config.run_id,
        episode_id: &config.episode_id,
        trajectory_id: &config.trajectory_id,
        trace_id: &config.trace_id,
        instance_id: &config.instance_id,
        session_id: &config.session_id,
        runtime_profile: &config.runtime_profile,
        provider_revision: &settings.exo.revision,
    })?;
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
            let _ = recording::flush_replay_stream();
            finish_telemetry(telemetry);
            return Err(error);
        }
    };
    if std::env::var("STS2_COMBAT_DEMO").as_deref() != Ok("true") {
        let path = std::env::var("STS2_REPLAY_TRAJECTORY").unwrap_or_default();
        if !path.is_empty() {
            let result = episode_replay::run(&mut port, &settings.runner, &path);
            drop(port);
            if let Ok(episode_replay::ReplayOutcome::Terminal(stage)) = result.as_ref() {
                let _ = telemetry_handle.run_finished(
                    recording::game_outcome(*stage),
                    TelemetryStage::from(*stage),
                    CleanupStatus::Clean,
                );
            } else if result.is_err() {
                let _ = telemetry_handle.failure(
                    "episode_replay",
                    super::runtime_v3_telemetry::FailureCode::Other,
                    false,
                    None,
                );
            }
            let _ = recording::flush_replay_stream();
            finish_telemetry(telemetry);
            return result.map(|_| ());
        }
    }
    let transport = ExoProcessTransport::new(settings.process);
    let provider = ExoProvider::new(transport, settings.exo);
    let mut source = ExoDecisionSource::new(ExoSession::new(provider));
    if std::env::var("STS2_COMBAT_DEMO").as_deref() == Ok("true") {
        let outcome = combat_demo::run(&mut port, &mut source, &settings.runner);
        let close = source.close().map_err(|error| error.to_string());
        drop(port);
        let mut completion = None;
        let result = match outcome {
            Ok(report) => {
                completion = Some((
                    report.steps(),
                    report.terminal_observation().stage(),
                    report.terminal_observation_digest(),
                ));
                let game_outcome = recording::game_outcome(report.terminal_observation().stage());
                recording::complete_observation(report.terminal_observation(), &telemetry_handle);
                let cleanup_status = if close.is_ok() {
                    CleanupStatus::Clean
                } else {
                    CleanupStatus::Failed
                };
                let _ = telemetry_handle.run_finished(
                    game_outcome,
                    TelemetryStage::from(report.terminal_observation().stage()),
                    cleanup_status,
                );
                match close {
                    Ok(()) => Ok(()),
                    Err(error) => Err(error),
                }
            }
            Err(failure) => {
                let cleanup_status =
                    if close.is_ok() && failure.cleanup_status() == CleanupStatus::Clean {
                        CleanupStatus::Clean
                    } else {
                        CleanupStatus::Failed
                    };
                if let Some(observation) = failure.terminal_observation() {
                    recording::complete_observation(observation, &telemetry_handle);
                    let _ = telemetry_handle.run_finished(
                        recording::game_outcome(observation.stage()),
                        TelemetryStage::from(observation.stage()),
                        cleanup_status,
                    );
                } else {
                    let _ = telemetry_handle.failure(
                        "combat_demo",
                        if cleanup_status == CleanupStatus::Failed {
                            super::runtime_v3_telemetry::FailureCode::Cleanup
                        } else {
                            super::runtime_v3_telemetry::FailureCode::Other
                        },
                        false,
                        None,
                    );
                }
                let mut message = failure.message().to_owned();
                if let Err(error) = close {
                    message.push_str(&format!("; provider cleanup failed: {error}"));
                }
                Err(message)
            }
        };
        let _ = recording::flush_replay_stream();
        finish_telemetry(telemetry);
        if let Some((steps, stage, terminal_observation_digest)) = completion {
            println!(
                "{}",
                json!({"event":"combat_demo_complete", "steps":steps,
                "stage":wire::stage_name(stage),
                "terminal_observation_digest":terminal_observation_digest})
            );
        }
        return result;
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
            recording::episode_failure(&error, &telemetry_handle);
            if source_close.is_err() {
                let _ = telemetry_handle.failure(
                    "provider_close",
                    super::runtime_v3_telemetry::FailureCode::Cleanup,
                    false,
                    None,
                );
            }
            // Preserve the runner's original, category-only error even if the private replay
            // sink has a broken pipe or another flush failure.
            let message = format!("Runtime-v3 episode failed: {error}");
            let _ = recording::flush_replay_stream();
            finish_telemetry(telemetry);
            return Err(message);
        }
    };
    let game_outcome = recording::game_outcome(report.terminal_stage());
    recording::complete(&report, &telemetry_handle);
    if source_close.is_err() {
        recording::cleanup_failure(&telemetry_handle);
        let _ = telemetry_handle.run_finished(
            game_outcome,
            TelemetryStage::from(report.terminal_stage()),
            CleanupStatus::Failed,
        );
        let _ = recording::flush_replay_stream();
        finish_telemetry(telemetry);
        return Err(String::from("Exo session close failed"));
    }
    let _ = telemetry_handle.run_finished(
        game_outcome,
        TelemetryStage::from(report.terminal_stage()),
        CleanupStatus::Clean,
    );
    let _ = recording::flush_replay_stream();
    finish_telemetry(telemetry);
    println!(
        "{}",
        serde_json::to_string(&json!({
            "protocol": runtime_profile,
            "status": "complete",
            "terminal_stage": wire::stage_name(report.terminal_stage()),
            "steps": report.steps(),
            "transitions": report.transitions(),
            "recoveries": report.recoveries(),
            "final_generation": report.final_observation().generation()
        }))
        .map_err(|error| format!("Runtime-v3 report serialization failed: {error}"))?
    );
    let _ = recording::flush_replay_stream();
    Ok(())
}

pub(super) struct RuntimeV3Port {
    config: RuntimeConfig,
    gateway: GatewayClient,
    mcp: Option<McpProcess>,
    expert_mcp: Option<McpProcess>,
    allocated: bool,
    released: bool,
    next_rpc_id: u64,
    expert_next_rpc_id: u64,
    generation: u64,
    current_state: Option<String>,
    current_actions: Option<EpisodeLegalActionSet>,
    payloads: BTreeMap<String, Value>,
    rest_selector_actions: Option<EpisodeLegalActionSet>,
    rest_selector_payloads: BTreeMap<String, Value>,
    rest_selector_value: Option<Value>,
    operations: BTreeMap<String, OperationRecord>,
    reconnect_attempts: u8,
    telemetry: TelemetryHandle,
}

include!("runtime_v3_port.rs");
