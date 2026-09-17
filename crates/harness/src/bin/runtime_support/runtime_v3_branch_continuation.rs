// SPDX-License-Identifier: MIT

//! Runtime-v3 dispatch for an explicitly selected durable branch continuation.

use sts2_harness::{
    Decision, DecisionInput, DecisionSource, EpisodeRunnerConfig, EpisodeStage, ExoDecisionSource,
    ExoProvider, ExoSession, ModelExecutionId, PolicyError,
};

use super::super::branch_continuation_runtime::BranchContinuationEffectPort;
use super::super::runtime_v3_telemetry::{
    CleanupStatus, RuntimeV3Telemetry, TelemetryHandle, TelemetryStage,
};
use super::super::{branch_continuation_runtime as branch_runtime, runtime_v3_telemetry};
use super::{RuntimeV3Port, episode_replay, recording, wire};

pub(super) fn run(
    mut selected: branch_runtime::SelectedBranchContinuation,
    mut port: RuntimeV3Port,
    settings: super::RuntimeV3Settings,
    telemetry_handle: TelemetryHandle,
    telemetry: RuntimeV3Telemetry,
) -> Result<(), String> {
    let claim = selected.prepare_owner_claim()?;
    if selected.is_exact_restore() && !selected.is_resuming() {
        selected.claim_exact_restore()?;
    } else if selected.is_resuming() {
        selected.claim_resume()?;
    }
    port.arm_continuation_owner_claim(
        super::continuation_owner::ContinuationOwnerClaimContext {
            branch_store_path: super::super::continuation_branch_store_path()?,
            claim,
        },
        selected.is_exact_restore(),
    )?;
    let preflight = if selected.is_resuming() {
        port.preflight_continuation_resume()
    } else {
        port.preflight_continuation_launch()
    };
    if let Err(error) = preflight {
        let _ = selected.mark_failed("exact/prefix continuation preflight failed");
        let durable_close = port
            .durable_handle()
            .map_or(Ok(()), |durable| durable.close());
        let _ = telemetry_handle.failure(
            "branch_continuation_owner",
            runtime_v3_telemetry::FailureCode::Configuration,
            false,
            None,
        );
        let _ = recording::flush_replay_stream();
        super::finish_telemetry(telemetry);
        drop(port);
        return Err(wire::combine_cleanup(error, Ok(()), durable_close));
    }
    let durable = port
        .durable_handle()
        .ok_or_else(|| String::from("runtime-v3 durable handle disappeared"))?;
    if selected.is_exact_restore() && !selected.is_resuming() {
        let receipt = match super::super::exact_restore::execute(&selected, &port.config) {
            Ok(receipt) => receipt,
            Err(error) => {
                let status = match error.safety {
                    super::super::exact_restore::FailureSafety::NotStarted => {
                        selected.mark_failed("exact restore was rejected before host mutation")
                    }
                    super::super::exact_restore::FailureSafety::Uncertain => {
                        selected.mark_unknown("exact restore effect or receipt is uncertain")
                    }
                };
                let cleanup =
                    if error.safety == super::super::exact_restore::FailureSafety::NotStarted {
                        port.cleanup_continuation_preflight()
                    } else {
                        Ok(())
                    };
                let durable_close = port
                    .durable_handle()
                    .map_or(Ok(()), |durable| durable.close());
                let _ = telemetry_handle.failure(
                    "exact_restore",
                    runtime_v3_telemetry::FailureCode::Other,
                    false,
                    None,
                );
                let _ = recording::flush_replay_stream();
                super::finish_telemetry(telemetry);
                drop(port);
                return Err(wire::combine_cleanup(
                    error.message,
                    status,
                    Err(wire::combine_cleanup(
                        String::from("exact-restore cleanup"),
                        cleanup,
                        durable_close,
                    )),
                ));
            }
        };
        if let Err(error) = selected.publish_exact_restore_receipt(&receipt) {
            let _ = selected.mark_unknown("verified receipt could not be persisted");
            let durable_close = port
                .durable_handle()
                .map_or(Ok(()), |durable| durable.close());
            drop(port);
            super::finish_telemetry(telemetry);
            return Err(wire::combine_cleanup(error, Ok(()), durable_close));
        }
        if let Err(error) = port.launch_gameplay_after_exact_restore() {
            let _ = selected.mark_failed("gameplay MCP could not start after verified restore");
            let durable_close = port
                .durable_handle()
                .map_or(Ok(()), |durable| durable.close());
            drop(port);
            super::finish_telemetry(telemetry);
            return Err(wire::combine_cleanup(error, Ok(()), durable_close));
        }
    }
    let transport = match super::select_provider_transport(
        &port.config,
        &settings,
        durable.clone(),
        port.lifecycle_authority_state(),
    ) {
        Ok(transport) => transport,
        Err(error) => {
            let _ = selected.mark_failed("provider admission failed");
            let cleanup = port.cleanup_continuation_preflight();
            let durable_close = port
                .durable_handle()
                .map_or(Ok(()), |durable| durable.close());
            let _ = telemetry_handle.failure(
                "branch_continuation_admission",
                runtime_v3_telemetry::FailureCode::Configuration,
                false,
                None,
            );
            let _ = recording::flush_replay_stream();
            super::finish_telemetry(telemetry);
            drop(port);
            return Err(wire::combine_cleanup(error, cleanup, durable_close));
        }
    };
    let mut source =
        ExoDecisionSource::new(ExoSession::new(ExoProvider::new(transport, settings.exo)));
    let mut recorder = recording::DecisionRecorder::with_durable(
        &mut source,
        telemetry_handle.clone(),
        durable.clone(),
    );
    let result = if selected.is_resuming() {
        sts2_harness::EpisodeRunner::new(settings.runner)
            .run(&mut port, &mut recorder)
            .map_err(|error| error.to_string())
            .and_then(terminal_result)
    } else {
        {
            let mut effect_port = RuntimeBranchContinuationEffectPort {
                port: &mut port,
                runner: &settings.runner,
                continuation: &mut recorder,
            };
            // The exact restore was committed and its verified receipt
            // persisted above. Its effect-port continuation only runs the
            // already owner-fenced gameplay destination; it never commits a
            // second restore.
            let outcome = if selected.is_exact_restore() {
                effect_port.exact_restore(&mut selected)
            } else {
                branch_runtime::dispatch(&mut selected, &mut effect_port)
            };
            outcome.and_then(|outcome| match outcome {
                episode_replay::ReplayOutcome::Terminal { stage, observation } => {
                    Ok((stage, observation))
                }
                episode_replay::ReplayOutcome::PrefixVerified => Err(String::from(
                    "branch replay reached its prefix boundary without continuing the live runner",
                )),
            })
        }
    };
    drop(recorder);
    let source_close = source.close();
    let result = match result {
        Ok((stage, observation)) => {
            if let Err(error) = durable.complete_observation(&observation) {
                let _ = selected.mark_failed("durable completion was uncertain");
                durable.mark_interrupted_unknown("branch continuation completion failed");
                Err(error)
            } else if let Err(error) = source_close {
                let _ = selected.mark_failed("provider cleanup was uncertain");
                durable.mark_interrupted_unknown("branch continuation provider close failed");
                recording::cleanup_failure(&telemetry_handle);
                Err(error.to_string())
            } else if let Err(error) = selected.complete() {
                durable.mark_interrupted_unknown("branch metadata completion failed");
                Err(error)
            } else {
                let _ = telemetry_handle.run_finished(
                    recording::game_outcome(stage),
                    TelemetryStage::from(stage),
                    CleanupStatus::Clean,
                );
                Ok(())
            }
        }
        Err(error) => {
            let _ = selected.mark_failed("runtime-v3 continuation failed");
            durable.mark_interrupted_unknown("runtime-v3 branch continuation failed");
            let _ = telemetry_handle.failure(
                "branch_continuation",
                runtime_v3_telemetry::FailureCode::Other,
                false,
                None,
            );
            if source_close.is_err() {
                recording::cleanup_failure(&telemetry_handle);
            }
            Err(error)
        }
    };
    let store_close = durable.close();
    drop(port);
    let result = result.and(store_close);
    let _ = recording::flush_replay_stream();
    super::finish_telemetry(telemetry);
    result
}

include!("runtime_v3_branch_continuation_effect.rs");
