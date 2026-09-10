// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use std::collections::BTreeMap;
use sts2_harness::{
    EpisodeLegalActionSet, EpisodeObservation, EpisodeRunner, ExoDecisionSource,
    ExoProcessTransport, ExoProvider, ExoSession, ResumeState, TransitionReceipt,
};

use super::config::RuntimeConfig;
use super::http::GatewayClient;
use super::mcp::{McpProcess, identity_headers, release_correlation};
use super::runtime_v3_parse as parse;
use super::runtime_v3_settings::RuntimeV3Settings;
use super::runtime_v3_telemetry::{
    CleanupStatus, GameOutcome, ObservationSource, RuntimeV3Telemetry, TelemetryContext,
    TelemetryContextInput, TelemetryHandle, TelemetryStage,
};
use super::runtime_v3_wire as wire;

#[path = "runtime_allocation_context.rs"]
mod allocation_context;
#[path = "runtime_v3_completed_resume.rs"]
mod completed_resume;
#[path = "runtime_v3_decision_admission.rs"]
mod decision_admission;
#[path = "runtime_v3_decision_replay.rs"]
mod decision_replay;
#[path = "runtime_v3_durable.rs"]
mod durable;

#[path = "runtime_v3_combat_demo.rs"]
pub(crate) mod combat_demo;
#[path = "runtime_v3_episode.rs"]
mod episode;
#[path = "runtime_v3_episode_replay.rs"]
mod episode_replay;
#[path = "runtime_v4_expert_port.rs"]
mod expert;
#[path = "runtime_v3_ledger.rs"]
mod ledger;
#[path = "runtime_v3_receipt_query.rs"]
#[allow(dead_code)]
mod receipt_query;
#[path = "runtime_v3_recording.rs"]
mod recording;
#[path = "runtime_v3_recovery.rs"]
mod recovery;
#[path = "runtime_map.rs"]
mod runtime_map;
#[path = "runtime_v3_seeded.rs"]
mod seeded;
#[path = "runtime_v3_seeded_receipt.rs"]
mod seeded_receipt;
#[path = "runtime_v3_seeded_validation.rs"]
mod seeded_validation;
#[path = "runtime_v3_shutdown.rs"]
mod shutdown;
#[path = "runtime_v3_wait.rs"]
mod wait;

#[cfg(test)]
#[path = "runtime_v3_lifecycle_test.rs"]
mod lifecycle_tests;

include!("runtime_v3_run_combat.rs");

pub(super) fn run(config: RuntimeConfig) -> Result<(), String> {
    let runtime_profile = config.runtime_profile.clone();
    let settings = RuntimeV3Settings::from_environment(&config)?;
    let resume_requested = std::env::args()
        .skip(1)
        .any(|argument| argument == "--resume")
        || std::env::var("STS2_RESUME").as_deref() == Ok("true");
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
            let _ = recording::flush_replay_stream();
            finish_telemetry(telemetry);
            return Err(error);
        }
    };
    if std::env::var("STS2_COMBAT_DEMO").as_deref() != Ok("true") {
        let path = std::env::var("STS2_REPLAY_TRAJECTORY").unwrap_or_default();
        if !path.is_empty() {
            let result = episode_replay::run(&mut port, &settings.runner, &path);
            let durable = port.durable_handle();
            match result.as_ref() {
                Ok(episode_replay::ReplayOutcome::Terminal { observation, .. }) => {
                    if let Some(durable) = &durable {
                        durable.complete_observation(observation)?;
                    }
                }
                Ok(episode_replay::ReplayOutcome::PrefixVerified) => {}
                Err(_) => {
                    if let Some(durable) = &durable {
                        durable.mark_interrupted_unknown("runtime-v3 episode replay failed");
                    }
                }
            }
            let store_close = durable
                .as_ref()
                .map_or(Ok(()), durable::DurableHandle::close);
            drop(port);
            if let Ok(episode_replay::ReplayOutcome::Terminal { stage, .. }) = result.as_ref() {
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
            return result.map(|_| ()).and(store_close);
        }
    }
    let transport = ExoProcessTransport::new(settings.process);
    let provider = ExoProvider::new(transport, settings.exo);
    let mut source = ExoDecisionSource::new(ExoSession::new(provider));
    if std::env::var("STS2_COMBAT_DEMO").as_deref() == Ok("true") {
        return run_combat_demo(port, source, settings.runner, telemetry_handle, telemetry);
    }
    let durable = port
        .durable_handle()
        .ok_or_else(|| String::from("runtime-v3 durable handle disappeared"))?;
    let mut recorder = recording::DecisionRecorder::with_durable(
        &mut source,
        telemetry_handle.clone(),
        durable.clone(),
    );
    let result = EpisodeRunner::new(settings.runner).run(&mut port, &mut recorder);
    drop(recorder);
    let source_close = source.close();
    let report = match result {
        Ok(report) => report,
        Err(error) => {
            durable.mark_interrupted_unknown("runtime-v3 episode failed");
            recording::episode_failure(&error, &telemetry_handle);
            if source_close.is_err() {
                let _ = telemetry_handle.failure(
                    "provider_close",
                    super::runtime_v3_telemetry::FailureCode::Cleanup,
                    false,
                    None,
                );
            }
            let message = format!("Runtime-v3 episode failed: {error}");
            let _ = durable.close();
            drop(port);
            let _ = recording::flush_replay_stream();
            finish_telemetry(telemetry);
            return Err(message);
        }
    };
    let game_outcome = recording::game_outcome(report.terminal_stage());
    recording::complete(&report, &telemetry_handle);
    if source_close.is_err() {
        durable.mark_interrupted_unknown("runtime-v3 provider close failed");
        recording::cleanup_failure(&telemetry_handle);
        let _ = durable.close();
        drop(port);
        let _ = telemetry_handle.run_finished(
            game_outcome,
            TelemetryStage::from(report.terminal_stage()),
            CleanupStatus::Failed,
        );
        let _ = recording::flush_replay_stream();
        finish_telemetry(telemetry);
        return Err(String::from("Exo session close failed"));
    }
    if let Err(error) = durable.complete_episode(&report) {
        durable.mark_interrupted_unknown("runtime-v3 durable completion failed");
        let _ = durable.close();
        drop(port);
        let _ = telemetry_handle.failure(
            "durable_completion",
            super::runtime_v3_telemetry::FailureCode::Other,
            false,
            None,
        );
        finish_telemetry(telemetry);
        return Err(error);
    }
    let store_close = durable.close();
    drop(port);
    if let Err(error) = store_close {
        finish_telemetry(telemetry);
        return Err(error);
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
    seeded_mcp: Option<McpProcess>,
    seeded_receipt: Option<Value>,
    expert_mcp: Option<McpProcess>,
    allocated: bool,
    released: bool,
    next_rpc_id: u64,
    expert_next_rpc_id: u64,
    generation: u64,
    current_state: Option<String>,
    current_actions: Option<EpisodeLegalActionSet>,
    catalog: Option<Value>,
    catalog_raw: Option<Vec<u8>>,
    payloads: BTreeMap<String, Value>,
    rest_selector_actions: Option<EpisodeLegalActionSet>,
    rest_selector_payloads: BTreeMap<String, Value>,
    rest_selector_value: Option<Value>,
    operations: BTreeMap<String, ledger::OperationRecord>,
    reconnect_attempts: u8,
    telemetry: TelemetryHandle,
    durable: Option<durable::DurableHandle>,
    last_response_text: Option<String>,
    recovery_authority: Option<allocation_context::RecoveryAuthority>,
    recovery: Option<McpProcess>,
    recovery_context: Option<recovery::RecoveryContext>,
    recovery_rpc_id: u64,
}

include!("runtime_v3_port.rs");
include!("runtime_v3_observation.rs");
