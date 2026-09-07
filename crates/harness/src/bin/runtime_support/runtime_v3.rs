// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use serde_json::{Value, json};
use sts2_harness::{
    Decision, EpisodeLegalActionSet, EpisodeObservation, EpisodeRunner, ExoDecisionSource,
    ExoProcessTransport, ExoProvider, ExoSession, ResumeState, ShutdownError, ShutdownPort,
};

use super::config::RuntimeConfig;
use super::http::GatewayClient;
use super::mcp::{McpProcess, identity_headers};
use super::runtime_v3_parse as parse;
use super::runtime_v3_settings::RuntimeV3Settings;
use super::runtime_v3_telemetry::{
    CleanupStatus, GameOutcome, RuntimeV3Telemetry, TelemetryContext, TelemetryContextLineage,
    TelemetryHandle, TelemetryStage,
};
use super::runtime_v3_wire as wire;

#[path = "runtime_v3_completed_resume.rs"]
mod completed_resume;
#[path = "runtime_v3_decision_replay.rs"]
mod decision_replay;
#[path = "runtime_v3_durable.rs"]
mod durable;
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

enum DecisionAdmission {
    Reused(Decision),
    Fresh(durable::ProviderReservationToken),
}

#[cfg(test)]
#[path = "runtime_v3_lifecycle_test.rs"]
mod lifecycle_tests;

pub(super) fn run(config: RuntimeConfig) -> Result<(), String> {
    let settings = RuntimeV3Settings::from_environment()?;
    let resume_requested = std::env::args()
        .skip(1)
        .any(|argument| argument == "--resume")
        || std::env::var("STS2_RESUME").as_deref() == Ok("true");
    let telemetry_context = TelemetryContext::new(
        TelemetryContextLineage {
            run_id: &config.run_id,
            episode_id: &config.episode_id,
            trajectory_id: &config.trajectory_id,
            trace_id: &config.trace_id,
        },
        &config.instance_id,
        &config.session_id,
        &config.runtime_profile,
        &settings.exo.revision,
    )?;
    let telemetry = RuntimeV3Telemetry::new(telemetry_context);
    let telemetry_handle = telemetry.handle();
    let _ = telemetry_handle.run_started();
    let (durable, state) = match durable::DurableHandle::open(&config, &settings, resume_requested)
    {
        Ok(result) => result,
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
    if let ResumeState::Completed(completion) = state {
        return completed_resume::finish(durable, completion, telemetry_handle, telemetry);
    }
    if resume_requested && let Err(error) = durable.validate_pending_action_identity() {
        let _ = telemetry_handle.failure(
            "runtime_resume",
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
    let mut port = match RuntimeV3Port::new_with_store(config, telemetry_handle.clone(), durable) {
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
            if result.is_err() {
                port.mark_interrupted_unknown("runtime-v3 episode replay failed");
            }
            let store_close = port.close_durable();
            drop(port);
            let _ = telemetry_handle.run_finished(
                if result.is_ok() {
                    GameOutcome::Success
                } else {
                    GameOutcome::Failure
                },
                TelemetryStage::Unknown,
                if store_close.is_ok() {
                    CleanupStatus::Clean
                } else {
                    CleanupStatus::Failed
                },
            );
            finish_telemetry(telemetry);
            return result.and(store_close);
        }
    }
    let transport = ExoProcessTransport::new(settings.process);
    let provider = ExoProvider::new(transport, settings.exo);
    let mut source = ExoDecisionSource::new(ExoSession::new(provider));
    if std::env::var("STS2_COMBAT_DEMO").as_deref() == Ok("true") {
        let durable_handle = port.durable_handle();
        let mut recorder =
            recording::DecisionRecorder::new(&mut source, telemetry_handle.clone(), durable_handle);
        let outcome = combat_demo::run(&mut port, &mut recorder, &settings.runner);
        if outcome.is_err() {
            port.mark_interrupted_unknown("runtime-v3 combat demo failed");
        }
        let close = source.close().map_err(|error| error.to_string());
        let store_close = port.close_durable();
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
        return outcome.and(close).and(store_close);
    }
    let durable_handle = port.durable_handle();
    let mut recorder =
        recording::DecisionRecorder::new(&mut source, telemetry_handle.clone(), durable_handle);
    let result = EpisodeRunner::new(settings.runner).run(&mut port, &mut recorder);
    let source_close = source.close();
    let report = match result {
        Ok(report) => report,
        Err(error) => {
            if source_close.is_err() {
                port.mark_interrupted_unknown("runtime-v3 episode and provider close failed");
            } else {
                port.mark_interrupted_unknown("runtime-v3 episode failed");
            }
            let _ = port.close_durable();
            drop(port);
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
        port.mark_interrupted_unknown("runtime-v3 provider close failed");
        let _ = port.close_durable();
        drop(port);
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
    if let Err(error) = port.complete_durable(&report) {
        port.mark_interrupted_unknown("runtime-v3 durable completion failed");
        let _ = port.close_durable();
        drop(port);
        let _ = telemetry_handle.failure(
            "durable_completion",
            super::runtime_v3_telemetry::FailureCode::Other,
            false,
            None,
        );
        let _ = telemetry_handle.run_finished(
            GameOutcome::Failure,
            TelemetryStage::from(report.terminal_stage()),
            CleanupStatus::Failed,
        );
        finish_telemetry(telemetry);
        return Err(error);
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
    let store_close = port.close_durable();
    if let Err(error) = store_close {
        drop(port);
        finish_telemetry(telemetry);
        return Err(error);
    }
    drop(port);
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
    catalog: Option<Value>,
    payloads: BTreeMap<String, Value>,
    operations: BTreeMap<String, OperationRecord>,
    reconnect_attempts: u8,
    telemetry: TelemetryHandle,
    durable: Option<durable::DurableHandle>,
    recovery: Option<McpProcess>,
    recovery_context: Option<recovery::RecoveryContext>,
    recovery_rpc_id: u64,
}

include!("runtime_v3_port.rs");
