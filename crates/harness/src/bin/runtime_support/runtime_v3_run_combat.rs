// SPDX-License-Identifier: MIT

fn run_combat_demo(
    mut port: RuntimeV3Port,
    mut source: ExoDecisionSource<ExoProcessTransport>,
    runner: sts2_harness::EpisodeRunnerConfig,
    telemetry_handle: TelemetryHandle,
    telemetry: RuntimeV3Telemetry,
) -> Result<(), String> {
    let durable = port
        .durable_handle()
        .ok_or_else(|| String::from("runtime-v3 durable handle disappeared"))?;
    let mut recorder = recording::DecisionRecorder::with_durable(
        &mut source,
        telemetry_handle.clone(),
        durable.clone(),
    );
    let outcome = combat_demo::run(&mut port, &mut recorder, &runner);
    drop(recorder);
    match outcome.as_ref() {
        Ok(report) if report.terminal_observation().stage().is_terminal() => {
            durable.complete_observation(report.terminal_observation())?;
        }
        Err(failure) => {
            if let Some(observation) = failure
                .terminal_observation()
                .filter(|observation| observation.stage().is_terminal())
            {
                durable.complete_observation(observation)?;
            } else {
                durable.mark_interrupted_unknown("runtime-v3 combat demo failed");
            }
        }
        Ok(_) => {}
    }
    let close = source.close().map_err(|error| error.to_string());
    let store_close = durable.close();
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
                Ok(()) => store_close,
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
            if let Err(error) = store_close {
                message.push_str(&format!("; durable store cleanup failed: {error}"));
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
    result
}
