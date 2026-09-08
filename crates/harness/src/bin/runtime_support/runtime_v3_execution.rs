// SPDX-License-Identifier: MIT

use serde_json::json;
use sts2_harness::{
    EpisodeRunner, ExoDecisionSource, ExoProcessTransport, ExoProvider, ExoSession, ResumeState,
};

use super::super::config::RuntimeConfig;
use super::super::runtime_v3_settings::RuntimeV3Settings;
use super::super::runtime_v3_telemetry::{
    CleanupStatus, FailureCode, GameOutcome, RuntimeV3Telemetry, TelemetryHandle, TelemetryStage,
};
use super::super::runtime_v3_wire as wire;
use super::combat_demo;
use super::completed_resume;
use super::durable::DurableHandle;
use super::episode_replay;
use super::launch_options::RuntimeV3LaunchOptions;
use super::recording;
use super::{RuntimeV3Port, finish_telemetry};

fn combine_quarantine(error: String, quarantine: Result<(), String>) -> String {
    match quarantine {
        Ok(()) => error,
        Err(quarantine_error) => {
            format!("{error}; failed to persist interrupted-unknown quarantine: {quarantine_error}")
        }
    }
}

/// Runs one already-admitted runtime-v3 episode.
///
/// All process-global launch selection is resolved by [`RuntimeV3LaunchOptions`] before this
/// function is called. In particular, this function never opens a second durable store and never
/// re-reads the environment while choosing a replay or gameplay branch.
pub(super) fn run(
    config: RuntimeConfig,
    settings: RuntimeV3Settings,
    durable: DurableHandle,
    state: ResumeState,
    options: RuntimeV3LaunchOptions,
    telemetry_handle: TelemetryHandle,
    telemetry: RuntimeV3Telemetry,
) -> Result<(), String> {
    if let ResumeState::Completed(completion) = state {
        return completed_resume::finish(durable, completion, telemetry_handle, telemetry);
    }
    if options.resume
        && let Err(error) = durable.validate_pending_action_identity()
    {
        let _ = telemetry_handle.failure("runtime_resume", FailureCode::Configuration, false, None);
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
            let _ =
                telemetry_handle.failure("runtime_init", FailureCode::Configuration, false, None);
            let _ = telemetry_handle.run_finished(
                GameOutcome::Unavailable,
                TelemetryStage::Unknown,
                CleanupStatus::Failed,
            );
            finish_telemetry(telemetry);
            return Err(error);
        }
    };
    if !options.combat_demo
        && let Some(path) = options.replay_path.as_deref()
    {
        let result = options
            .episode_replay_prefix()
            .and_then(|prefix| episode_replay::run(&mut port, &settings.runner, path, prefix));
        let quarantine = if result.is_err() {
            port.mark_interrupted_unknown("runtime-v3 episode replay failed")
        } else {
            Ok(())
        };
        let result = result.map_err(|error| combine_quarantine(error, quarantine));
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
    let transport = ExoProcessTransport::new(settings.process);
    let provider = ExoProvider::new(transport, settings.exo);
    let mut source = ExoDecisionSource::new(ExoSession::new(provider));
    if options.combat_demo {
        let durable_handle = port.durable_handle();
        let mut recorder =
            recording::DecisionRecorder::new(&mut source, telemetry_handle.clone(), durable_handle);
        let outcome = combat_demo::run(
            &mut port,
            &mut recorder,
            &settings.runner,
            options.replay_path.as_deref(),
        );
        let quarantine = if outcome.is_err() {
            port.mark_interrupted_unknown("runtime-v3 combat demo failed")
        } else {
            Ok(())
        };
        let outcome = outcome.map_err(|error| combine_quarantine(error, quarantine));
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
            let quarantine = if source_close.is_err() {
                port.mark_interrupted_unknown("runtime-v3 episode and provider close failed")
            } else {
                port.mark_interrupted_unknown("runtime-v3 episode failed")
            };
            let error =
                combine_quarantine(format!("Runtime-v3 episode failed: {error}"), quarantine);
            let _ = port.close_durable();
            drop(port);
            let _ = telemetry_handle.failure("episode", FailureCode::Other, false, None);
            let _ = telemetry_handle.run_finished(
                GameOutcome::Unavailable,
                TelemetryStage::Unknown,
                CleanupStatus::Failed,
            );
            finish_telemetry(telemetry);
            return Err(error);
        }
    };
    if source_close.is_err() {
        let quarantine = port.mark_interrupted_unknown("runtime-v3 provider close failed");
        let _ = port.close_durable();
        drop(port);
        let _ = telemetry_handle.failure("provider_close", FailureCode::Cleanup, false, None);
        let _ = telemetry_handle.run_finished(
            GameOutcome::Unavailable,
            TelemetryStage::from(report.terminal_stage()),
            CleanupStatus::Failed,
        );
        finish_telemetry(telemetry);
        return Err(combine_quarantine(
            String::from("Exo session close failed"),
            quarantine,
        ));
    }
    if let Err(error) = port.complete_durable(&report) {
        let quarantine = port.mark_interrupted_unknown("runtime-v3 durable completion failed");
        let _ = port.close_durable();
        drop(port);
        let _ = telemetry_handle.failure("durable_completion", FailureCode::Other, false, None);
        let _ = telemetry_handle.run_finished(
            GameOutcome::Failure,
            TelemetryStage::from(report.terminal_stage()),
            CleanupStatus::Failed,
        );
        finish_telemetry(telemetry);
        return Err(combine_quarantine(error, quarantine));
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

#[cfg(test)]
#[path = "runtime_v3_execution_tests.rs"]
mod tests;
