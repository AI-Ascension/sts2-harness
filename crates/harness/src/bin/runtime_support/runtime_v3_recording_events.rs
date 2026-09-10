// SPDX-License-Identifier: MIT

pub(super) fn receipt(receipt: &TransitionReceipt, generation: u64, telemetry: &TelemetryHandle) {
    let failure_code = match receipt.status() {
        DispatchStatus::Rejected | DispatchStatus::Cancelled => Some(FailureCode::Rejected),
        DispatchStatus::Unknown => Some(FailureCode::UnknownOutcome),
        DispatchStatus::Accepted | DispatchStatus::Settled => None,
    };
    let _ = telemetry.action_dispatch(
        receipt.operation_id(),
        receipt.action().action_id(),
        receipt.action().kind(),
        generation,
        receipt.status(),
        failure_code,
    );
    emit_replay_event(json!({
        "event": "action_receipt",
        "operation_id": receipt.operation_id(),
        "action_id": receipt.action().action_id(),
        "status": dispatch_status_name(receipt.status()),
        "effect": receipt.effect_kind(),
        "observation": receipt
            .after()
            .map_or(serde_json::Value::Null, |after| after.fair_play().as_value().clone())
    }));
    if receipt.status() == DispatchStatus::Settled
        && receipt
            .after()
            .is_some_and(|after| after.generation() > generation)
        && receipt.effect_kind().is_some()
        && let Some(after) = receipt.after()
        && let Some(effect_kind) = receipt.effect_kind()
    {
        let _ = telemetry.settlement(
            receipt.operation_id(),
            receipt.action().action_id(),
            generation,
            after,
            effect_kind,
            ObservationSource::Transition,
        );
    }
}

pub(super) fn wait(
    operation_id: &str,
    action_id: &str,
    generation: u64,
    sample: &WaitSample,
    telemetry: &TelemetryHandle,
) {
    let source = match sample.outcome() {
        WaitOutcome::Successor | WaitOutcome::SameStateMutation => ObservationSource::Transition,
        WaitOutcome::Timeout => return,
        WaitOutcome::RecoveryRequired => ObservationSource::Recovery,
    };
    if let Some(after) = sample.observation()
        && after.generation() > generation
        && let Some(effect_kind) = sample.effect_kind()
    {
        let _ = telemetry.settlement(
            operation_id,
            action_id,
            generation,
            after,
            effect_kind,
            source,
        );
        emit_replay_event(json!({
            "event": "operation_wait_completed",
            "operation_id": operation_id,
            "action_id": action_id,
            "effect": effect_kind,
            "observation": after.fair_play().as_value()
        }));
    }
}

pub(super) fn game_outcome(stage: sts2_harness::EpisodeStage) -> GameOutcome {
    match stage {
        sts2_harness::EpisodeStage::Victory | sts2_harness::EpisodeStage::Reward => {
            GameOutcome::Success
        }
        sts2_harness::EpisodeStage::Defeat => GameOutcome::Failure,
        _ => GameOutcome::Unavailable,
    }
}

pub(super) fn complete_observation(observation: &EpisodeObservation, telemetry: &TelemetryHandle) {
    let outcome = game_outcome(observation.stage());
    let _ = telemetry.terminal(observation, outcome);
}

pub(super) fn complete(report: &EpisodeRunReport, telemetry: &TelemetryHandle) {
    let outcome = game_outcome(report.terminal_stage());
    let _ = telemetry.terminal(report.final_observation(), outcome);
    emit_replay_event(json!({
        "event": "episode_complete",
        "steps": report.steps(),
        "transitions": report.transitions(),
        "recoveries": report.recoveries(),
        "observation": report.final_observation().fair_play().as_value()
    }));
}

pub(super) fn episode_failure(error: &EpisodeRunnerError, telemetry: &TelemetryHandle) {
    // Keep the OTLP failure vocabulary stable. The replay stream carries its own finite
    // map-diagnostic code so this additive evidence does not change the telemetry allowlist.
    let _ = telemetry.failure("episode", FailureCode::Other, false, None);
    emit_replay_event(json!({
        "event": "episode_failed",
        "error_code": replay_failure_code(error)
    }));
}

pub(super) fn cleanup_failure(telemetry: &TelemetryHandle) {
    let code = FailureCode::Cleanup;
    let _ = telemetry.failure("provider_close", code, false, None);
    emit_replay_event(json!({
        "event": "episode_failed",
        "error_code": "cleanup"
    }));
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReplayFailureCode {
    MapSnapshotInvalid,
    Other,
}

impl ReplayFailureCode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::MapSnapshotInvalid => "map_snapshot_invalid",
            Self::Other => "other",
        }
    }
}

fn replay_failure_code(error: &EpisodeRunnerError) -> &'static str {
    match error {
        EpisodeRunnerError::LegalActions(error) if error.code() == "map_snapshot_invalid" => {
            ReplayFailureCode::MapSnapshotInvalid.as_str()
        }
        _ => ReplayFailureCode::Other.as_str(),
    }
}

const fn dispatch_status_name(status: DispatchStatus) -> &'static str {
    match status {
        DispatchStatus::Accepted => "Accepted",
        DispatchStatus::Settled => "Settled",
        DispatchStatus::Rejected => "Rejected",
        DispatchStatus::Unknown => "Unknown",
        DispatchStatus::Cancelled => "Cancelled",
    }
}
