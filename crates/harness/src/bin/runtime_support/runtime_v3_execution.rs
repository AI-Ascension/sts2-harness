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

#[path = "runtime_v3_execution_cleanup.rs"]
mod cleanup;
use cleanup::{
    combine_quarantine, combine_store_close, finish_cleanup, quarantine_unless_completed,
};

#[path = "runtime_v3_execution_telemetry.rs"]
mod execution_telemetry;
use execution_telemetry::combat_demo_telemetry;

fn combat_demo_complete_event(
    steps: u32,
    stage: sts2_harness::EpisodeStage,
    terminal_observation_digest: String,
) -> serde_json::Value {
    json!({
        "event": "combat_demo_complete",
        "steps": steps,
        "stage": wire::stage_name(stage),
        "terminal_observation_digest": terminal_observation_digest,
    })
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
    port.cancellation = options.cancellation.clone();
    port.gateway.set_cancellation(options.cancellation.clone());
    if !options.combat_demo && options.replay_path.is_some() {
        let result = options.episode_replay_prefix().and_then(|prefix| {
            let bytes = options.replay_bytes.as_deref().ok_or_else(|| {
                String::from("runtime-v3 replay bytes were not captured before admission")
            })?;
            episode_replay::run_with_bytes(&mut port, &settings.runner, bytes, prefix)
        });
        let quarantine = if result.is_err() {
            quarantine_unless_completed(&port, "runtime-v3 episode replay failed")
        } else {
            Ok(())
        };
        let result = result.map_err(|error| combine_quarantine(error, quarantine));
        let store_close = port.close_durable();
        drop(port);
        let (game_outcome, telemetry_stage) = match result.as_ref().ok() {
            Some(episode_replay::ReplayOutcome::Terminal(stage)) => (
                recording::game_outcome(*stage),
                TelemetryStage::from(*stage),
            ),
            Some(episode_replay::ReplayOutcome::PrefixVerified) => {
                (GameOutcome::Unavailable, TelemetryStage::Recovery)
            }
            None => (GameOutcome::Failure, TelemetryStage::Unknown),
        };
        let _ = telemetry_handle.run_finished(
            game_outcome,
            telemetry_stage,
            if store_close.is_ok() {
                CleanupStatus::Clean
            } else {
                CleanupStatus::Failed
            },
        );
        finish_telemetry(telemetry);
        return finish_cleanup(
            result.map(|_| ()),
            store_close,
            "execution store close failed",
        );
    }
    let transport =
        ExoProcessTransport::new(settings.process).with_cancellation(options.cancellation.clone());
    let provider = ExoProvider::new(transport, settings.exo);
    let mut source = ExoDecisionSource::new(ExoSession::new(provider));
    if options.combat_demo {
        let durable_handle = port.durable_handle();
        let mut recorder =
            recording::DecisionRecorder::new(&mut source, telemetry_handle.clone(), durable_handle);
        let (outcome, terminal_stage, workflow_cleanup) = match combat_demo::run_with_replay_bytes(
            &mut port,
            &mut recorder,
            &settings.runner,
            options.replay_bytes.as_deref(),
        ) {
            Ok(report) => {
                let terminal_stage = report.terminal_observation().stage();
                (Ok(report), Some(terminal_stage), CleanupStatus::Clean)
            }
            Err(failure) => {
                let terminal_stage = failure
                    .terminal_observation()
                    .map(|observation| observation.stage());
                let workflow_cleanup = failure.cleanup_status();
                let quarantine = if failure.needs_quarantine() {
                    port.mark_interrupted_unknown("runtime-v3 combat demo failed")
                } else {
                    Ok(())
                };
                (
                    Err(combine_quarantine(failure.message().to_owned(), quarantine)),
                    terminal_stage,
                    workflow_cleanup,
                )
            }
        };
        // `Ok(report)` means the combat workflow crossed its durable completion boundary and
        // its own shutdown policy. Keep the completion event outside the workflow so it cannot be
        // emitted when durable completion itself failed.
        let completion = outcome.as_ref().ok().map(|report| {
            (
                report.steps(),
                report.terminal_observation().stage(),
                report.terminal_observation_digest(),
            )
        });
        let close = source.close().map_err(|error| error.to_string());
        let store_close = port.close_durable();
        drop(port);
        let summary = combat_demo_telemetry(
            terminal_stage,
            workflow_cleanup,
            close.is_ok(),
            store_close.is_ok(),
        );
        let _ = telemetry_handle.run_finished(
            summary.game_outcome,
            summary.terminal_stage,
            summary.cleanup_status,
        );
        finish_telemetry(telemetry);
        let outcome = finish_cleanup(outcome.map(|_| ()), close, "provider close failed");
        let result = finish_cleanup(outcome, store_close, "execution store close failed");
        if result.is_ok()
            && let Some((steps, stage, terminal_observation_digest)) = completion
        {
            println!(
                "{}",
                combat_demo_complete_event(steps, stage, terminal_observation_digest)
            );
        }
        return result;
    }
    let durable_handle = port.durable_handle();
    let mut recorder =
        recording::DecisionRecorder::new(&mut source, telemetry_handle.clone(), durable_handle);
    let result = EpisodeRunner::new(settings.runner).run_with_completion(
        &mut port,
        &mut recorder,
        |port, _, report| {
            port.complete_durable(report)
                .map_err(|error| wire::port_error("durable_completion", error, false))
        },
    );
    let source_close = source.close();
    let report = match result {
        Ok(report) => report,
        Err(error) => {
            let quarantine = if source_close.is_err() {
                quarantine_unless_completed(&port, "runtime-v3 episode and provider close failed")
            } else {
                quarantine_unless_completed(&port, "runtime-v3 episode failed")
            };
            let error =
                combine_quarantine(format!("Runtime-v3 episode failed: {error}"), quarantine);
            let error = combine_store_close(error, port.close_durable());
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
        let quarantine = quarantine_unless_completed(&port, "runtime-v3 provider close failed");
        let error = combine_store_close(
            combine_quarantine(String::from("Exo session close failed"), quarantine),
            port.close_durable(),
        );
        drop(port);
        let _ = telemetry_handle.failure("provider_close", FailureCode::Cleanup, false, None);
        let _ = telemetry_handle.run_finished(
            GameOutcome::Unavailable,
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
    let store_close = port.close_durable();
    let cleanup = if store_close.is_ok() {
        CleanupStatus::Clean
    } else {
        CleanupStatus::Failed
    };
    let _ = telemetry_handle.run_finished(
        game_outcome,
        TelemetryStage::from(report.terminal_stage()),
        cleanup,
    );
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
