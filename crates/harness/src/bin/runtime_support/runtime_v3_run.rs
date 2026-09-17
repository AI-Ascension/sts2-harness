// SPDX-License-Identifier: MIT

pub(super) fn run(
    mut config: RuntimeConfig,
    selector: Option<sts2_harness::BranchContinuationSelector>,
) -> Result<(), String> {
    let runtime_profile = config.runtime_profile.clone();
    let resume_requested = std::env::args()
        .skip(1)
        .any(|argument| argument == "--resume")
        || std::env::var("STS2_RESUME").as_deref() == Ok("true");
    let mut selected_branch =
        select_branch_continuation(selector, resume_requested, &mut config)?;
    let policy_preflight = if selected_branch
        .as_ref()
        .is_some_and(|selected| selected.is_exact_restore())
    {
        None
    } else {
        game_information_owner::begin_memory_policy_preflight(&config)?
    };
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
    if let Some(selected) = selected_branch
        .as_ref()
        .filter(|selected| selected.is_resuming())
    {
        let verification = selected
            .branch()
            .effective_seed
            .as_deref()
            .ok_or_else(|| String::from("selected running branch has no effective seed"))
            .and_then(|branch_seed| durable.verify_branch_effective_seed(branch_seed));
        if let Err(error) = verification {
            let close = durable.close();
            let _ = telemetry_handle.failure(
                "branch_continuation_seed",
                super::runtime_v3_telemetry::FailureCode::Configuration,
                false,
                None,
            );
            let _ = telemetry_handle.run_finished(
                GameOutcome::Unavailable,
                TelemetryStage::Unknown,
                if close.is_ok() {
                    CleanupStatus::Clean
                } else {
                    CleanupStatus::Failed
                },
            );
            finish_telemetry(telemetry);
            return Err(match close {
                Ok(()) => error,
                Err(close_error) => {
                    format!("{error}; execution store cleanup failed: {close_error}")
                }
            });
        }
    }
    if let ResumeState::Completed(completion) = state {
        if let Some(mut selected) = selected_branch.take() {
            if !selected.is_resuming() {
                return Err(String::from(
                    "a completed execution store cannot start a new selected branch continuation",
                ));
            }
            selected.complete()?;
        }
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
    let mut port =
        match RuntimeV3Port::new_with_store_and_lookup_owner(
            config.clone(),
            telemetry_handle.clone(),
            durable,
            policy_preflight
                .as_ref()
                .map(|preflight| std::sync::Arc::clone(&preflight.owner)),
        ) {
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
    if let Some(selected) = selected_branch.take() {
        return branch_continuation::run(selected, port, settings, telemetry_handle, telemetry);
    }
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
    if std::env::var("STS2_COMBAT_DEMO").as_deref() == Ok("true") {
        let transport = select_provider_transport(
            &config,
            &settings,
            port.durable_handle()
                .ok_or_else(|| String::from("runtime-v3 durable handle disappeared"))?,
            port.lifecycle_authority_state(),
        )?;
        let provider = ExoProvider::new(transport, settings.exo.clone());
        let source = ExoDecisionSource::new(ExoSession::new(provider));
        return run_combat_demo(port, source, settings.runner, telemetry_handle, telemetry);
    }
    let mut source = decision_source(
        &config,
        &settings,
        port.durable_handle()
            .ok_or_else(|| String::from("runtime-v3 durable handle disappeared"))?,
        port.lifecycle_authority_state(),
        policy_preflight.is_some(),
    )?;
    let durable = port
        .durable_handle()
        .ok_or_else(|| String::from("runtime-v3 durable handle disappeared"))?;
    let mut recorder = if settings.lifecycle.is_some() {
        recording::DecisionRecorder::new(&mut *source, telemetry_handle.clone())
    } else {
        recording::DecisionRecorder::with_durable(
            &mut *source,
            telemetry_handle.clone(),
            durable.clone(),
        )
    };
    let result = EpisodeRunner::new(settings.runner).run(&mut port, &mut recorder);
    drop(recorder);
    let source_close = source.close();
    let report = match result {
        Ok(report) => report,
        Err(error) => {
            if std::env::var("STS2_LIVE_EPISODE").as_deref() == Ok("true") {
                eprintln!("runtime-v3 episode primary error: {error:?}");
            }
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

include!("runtime_v3_run_transport.rs");
