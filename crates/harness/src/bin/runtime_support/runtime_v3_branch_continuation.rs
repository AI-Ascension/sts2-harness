// SPDX-License-Identifier: MIT

//! Runtime-v3 dispatch for an explicitly selected durable branch continuation.

use sts2_harness::{
    DecisionSource, EpisodeRunnerConfig, EpisodeStage, ExoDecisionSource, ExoProvider, ExoSession,
};

use super::super::branch_continuation_runtime::BranchContinuationEffectPort;
use super::super::runtime_v3_telemetry::{
    CleanupStatus, RuntimeV3Telemetry, TelemetryHandle, TelemetryStage,
};
use super::super::{
    branch_continuation_runtime as branch_runtime, runtime_v3_admission, runtime_v3_telemetry,
};
use super::{RuntimeV3Port, episode_replay, recording, wire};

pub(super) fn run(
    mut selected: branch_runtime::SelectedBranchContinuation,
    mut port: RuntimeV3Port,
    settings: super::RuntimeV3Settings,
    telemetry_handle: TelemetryHandle,
    telemetry: RuntimeV3Telemetry,
) -> Result<(), String> {
    let claim = selected.prepare_owner_claim()?;
    port.arm_continuation_owner_claim(super::continuation_owner::ContinuationOwnerClaimContext {
        branch_store_path: super::super::continuation_branch_store_path()?,
        claim,
    })?;
    if let Err(error) = port.preflight_continuation_launch() {
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
    let transport = match runtime_v3_admission::admit(&settings.admission, settings.process) {
        Ok(transport) => transport,
        Err(error) => {
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
    let durable = port
        .durable_handle()
        .ok_or_else(|| String::from("runtime-v3 durable handle disappeared"))?;
    let mut recorder = recording::DecisionRecorder::with_durable(
        &mut source,
        telemetry_handle.clone(),
        durable.clone(),
    );
    let result = if selected.is_resuming() {
        sts2_harness::EpisodeRunner::new(settings.runner)
            .run(&mut port, &mut recorder)
            .map(|report| (report.terminal_stage(), report.final_observation().clone()))
            .map_err(|error| error.to_string())
    } else {
        {
            let mut effect_port = RuntimeBranchContinuationEffectPort {
                port: &mut port,
                runner: &settings.runner,
                continuation: &mut recorder,
            };
            branch_runtime::dispatch(&mut selected, &mut effect_port).and_then(|outcome| {
                match outcome {
                    episode_replay::ReplayOutcome::Terminal { stage, observation } => {
                        Ok((stage, observation))
                    }
                    episode_replay::ReplayOutcome::PrefixVerified => Err(String::from(
                        "branch replay reached its prefix boundary without continuing the live runner",
                    )),
                }
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

struct RuntimeBranchContinuationEffectPort<'a> {
    port: &'a mut RuntimeV3Port,
    runner: &'a EpisodeRunnerConfig,
    continuation: &'a mut dyn DecisionSource,
}

impl BranchContinuationEffectPort for RuntimeBranchContinuationEffectPort<'_> {
    type Output = episode_replay::ReplayOutcome;

    fn exact_restore(
        &mut self,
        _selected: &mut branch_runtime::SelectedBranchContinuation,
    ) -> Result<Self::Output, String> {
        Err(String::from(
            "exact branch continuation is unavailable: no fixed game-mod/MCP/gateway restore route is installed",
        ))
    }

    fn prefix_replay(
        &mut self,
        selected: &mut branch_runtime::SelectedBranchContinuation,
        prefix: &[u8],
    ) -> Result<Self::Output, String> {
        selected.claim_prefix_replay()?;
        let mut publish_boundary = || selected.publish_prefix_boundary();
        let outcome = episode_replay::run_prefix_and_continue(
            self.port,
            self.runner,
            prefix,
            self.continuation,
            &mut publish_boundary,
        )?;
        if matches!(
            outcome,
            episode_replay::ReplayOutcome::Terminal {
                stage: EpisodeStage::Unknown,
                ..
            }
        ) {
            return Err(String::from(
                "branch continuation ended at an unknown gameplay stage",
            ));
        }
        Ok(outcome)
    }
}
