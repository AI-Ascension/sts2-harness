// SPDX-License-Identifier: MIT

use serde_json::json;
use sts2_harness::{
    Decision, DecisionInput, DecisionSource, DispatchStatus, EpisodeObservation, EpisodeRunReport,
    EpisodeRunnerError, PolicyError, TransitionReceipt, WaitOutcome, WaitSample,
};

use super::super::runtime_v3_telemetry::{
    DecisionKind, FailureCode, GameOutcome, ObservationSource, TelemetryHandle,
};

const MAX_REPLAY_EVENT_BYTES: usize = 512 * 1024;

#[cfg(test)]
use std::cell::RefCell;

#[cfg(test)]
thread_local! {
    static REPLAY_CAPTURE: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
}

fn replay_enabled() -> bool {
    std::env::var("STS2_LIVE_EPISODE").as_deref() == Ok("true")
}

fn encode_replay_event(event: &serde_json::Value) -> Option<Vec<u8>> {
    let mut bytes = serde_json::to_vec(event).ok()?;
    if bytes.len().checked_add(1)? > MAX_REPLAY_EVENT_BYTES {
        return None;
    }
    bytes.push(b'\n');
    Some(bytes)
}

fn emit_replay_event(event: serde_json::Value) {
    let bytes = encode_replay_event(&event).or_else(|| {
        // A bounded event must never disappear silently: the marker is intentionally a
        // parser-failing record so a truncated stream cannot masquerade as a complete replay.
        let marker = json!({
            "event": "replay_stream_truncated",
            "source_event": replay_event_name(&event),
            "reason": "event_exceeds_bound"
        });
        encode_replay_event(&marker)
    });
    let Some(bytes) = bytes else {
        return;
    };
    emit_replay_bytes(&bytes);
}

fn replay_event_name(event: &serde_json::Value) -> &'static str {
    match event["event"].as_str() {
        Some("model_decision") => "model_decision",
        Some("action_receipt") => "action_receipt",
        Some("operation_wait_completed") => "operation_wait_completed",
        Some("episode_complete") => "episode_complete",
        Some("episode_failed") => "episode_failed",
        _ => "unknown",
    }
}

fn emit_replay_bytes(bytes: &[u8]) {
    #[cfg(test)]
    if REPLAY_CAPTURE.with(|capture| {
        let mut capture = capture.borrow_mut();
        if let Some(output) = capture.as_mut() {
            output.extend_from_slice(bytes);
            true
        } else {
            false
        }
    }) {
        return;
    }
    if !replay_enabled() {
        return;
    }
    let mut stdout = std::io::stdout().lock();
    let _ = std::io::Write::write_all(&mut stdout, bytes);
}

/// Captures the same bounded event bytes used by the stdout sink for deterministic integration
/// tests. The hook is test-only and thread-local so it cannot alter a live worker's stream.
#[cfg(test)]
pub(super) fn capture_replay_events<F, T>(operation: F) -> (T, Vec<u8>)
where
    F: FnOnce() -> T,
{
    REPLAY_CAPTURE.with(|capture| {
        assert!(
            capture.borrow().is_none(),
            "replay capture was already active"
        );
        *capture.borrow_mut() = Some(Vec::new());
    });
    let result = operation();
    let bytes = REPLAY_CAPTURE.with(|capture| capture.borrow_mut().take().unwrap_or_default());
    (result, bytes)
}

/// Flushes the private replay stream without changing the episode's result when the stream
/// itself cannot be flushed. The caller deliberately keeps the original failure authoritative.
pub(super) fn flush_replay_stream() -> std::io::Result<()> {
    if replay_enabled() {
        std::io::Write::flush(&mut std::io::stdout().lock())
    } else {
        Ok(())
    }
}

pub(super) struct DecisionRecorder<'a, S> {
    source: &'a mut S,
    telemetry: TelemetryHandle,
}

impl<'a, S> DecisionRecorder<'a, S> {
    pub(super) fn new(source: &'a mut S, telemetry: TelemetryHandle) -> Self {
        Self { source, telemetry }
    }
}

impl<S: DecisionSource> DecisionSource for DecisionRecorder<'_, S> {
    fn model_execution_id(&self) -> Option<sts2_harness::ModelExecutionId> {
        self.source.model_execution_id()
    }

    fn action_completed(&mut self, settled: bool) {
        self.source.action_completed(settled);
    }

    fn decide(&mut self, input: &DecisionInput) -> Result<Decision, PolicyError> {
        let decision = match self.source.decide(input) {
            Ok(decision) => decision,
            Err(error) => {
                let execution_id = self
                    .source
                    .model_execution_id()
                    .unwrap_or(input.execution_id);
                let _ = self
                    .telemetry
                    .model_failure(execution_id.get(), FailureCode::from(&error));
                return Err(error);
            }
        };
        let execution_id = self
            .source
            .model_execution_id()
            .unwrap_or(input.execution_id);
        // ExoDecisionSource expands a supported provider Plan into one Action at a time while
        // retaining its originating execution ID. Recording the post-expansion choice here
        // gives replay one correspondence row per dispatch, including reused plan executions.
        if let Decision::Action { action_id, .. } = &decision {
            emit_replay_event(json!({
                "event": "model_decision",
                "model_execution_id": execution_id.get(),
                "reused_model_execution": execution_id != input.execution_id,
                "action_id": action_id,
                "observation": input.observation.fair_play().as_value()
            }));
        }
        let (kind, action_id, operation_id, confidence) = match &decision {
            Decision::Plan { action_ids, .. } => (
                DecisionKind::Plan,
                action_ids.first().map(String::as_str),
                None,
                None,
            ),
            Decision::Action {
                action_id,
                confidence,
                ..
            } => (
                DecisionKind::Action,
                Some(action_id.as_str()),
                None,
                *confidence,
            ),
            Decision::Wait { .. } => (DecisionKind::Wait, None, None, None),
            Decision::Reobserve { .. } => (DecisionKind::Reobserve, None, None, None),
            Decision::Recovery { operation_id, .. } => {
                (DecisionKind::Recovery, None, operation_id.as_deref(), None)
            }
        };
        let _ = self.telemetry.model_decision(
            execution_id.get(),
            kind,
            action_id,
            operation_id,
            confidence,
        );
        Ok(decision)
    }
}

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

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use std::collections::VecDeque;

    use super::{
        DecisionRecorder, GameOutcome, MAX_REPLAY_EVENT_BYTES, capture_replay_events,
        dispatch_status_name, emit_replay_event, encode_replay_event, game_outcome,
        replay_failure_code,
    };
    use serde_json::json;
    use sts2_harness::{
        ActionKind, DecisionInput, DecisionSource, EpisodeLegalAction, EpisodeLegalActionSet,
        EpisodeObservation, EpisodeRunnerError, EpisodeStage, ExoConfig, ExoDecisionSource,
        ExoProvider, ExoSession, ExoTransport, ExoTransportError, ModelExecutionId, PortError,
    };

    #[test]
    fn terminal_stage_outcome_survives_independent_cleanup_status() {
        assert_eq!(game_outcome(EpisodeStage::Victory), GameOutcome::Success);
        assert_eq!(game_outcome(EpisodeStage::Reward), GameOutcome::Success);
        assert_eq!(game_outcome(EpisodeStage::Defeat), GameOutcome::Failure);
        assert_eq!(game_outcome(EpisodeStage::Combat), GameOutcome::Unavailable);
    }

    #[test]
    fn failed_episode_after_settled_action_keeps_a_safe_distinct_code() {
        let error = EpisodeRunnerError::LegalActions(PortError::new(
            "map_snapshot_invalid",
            "PRIVATE_MAP_DIAGNOSTIC",
            false,
        ));
        assert_eq!(replay_failure_code(&error), "map_snapshot_invalid");
        assert_eq!(
            dispatch_status_name(sts2_harness::DispatchStatus::Settled),
            "Settled"
        );
        let event = json!({"event":"episode_failed", "error_code":replay_failure_code(&error)});
        let text = event.to_string();
        assert!(!text.contains("PRIVATE_MAP_DIAGNOSTIC"));
        assert!(!text.contains("map_snapshot_failed"));
    }

    #[test]
    fn replay_event_encoding_is_bounded_and_contains_only_fair_play_fields() {
        let event = json!({
            "event":"model_decision",
            "model_execution_id":7,
            "reused_model_execution":false,
            "action_id":"start_run",
            "observation": {
                "state_id":"setup-1",
                "generation":1,
                "visible_seed":"SEED",
                "player":{"hp":80,"max_hp":80},
                "state":{"state":"setup"},
                "legal_actions":[]
            }
        });
        let bytes = encode_replay_event(&event).expect("small replay event");
        assert!(bytes.len() <= MAX_REPLAY_EVENT_BYTES);
        let text = String::from_utf8(bytes).expect("event UTF-8");
        assert!(text.ends_with('\n'));
        assert!(!text.contains("rationale"));
        assert!(!text.contains("provider_output"));

        let oversized = json!({"event":"episode_complete", "padding":"x".repeat(
            MAX_REPLAY_EVENT_BYTES
        )});
        assert!(encode_replay_event(&oversized).is_none());
    }

    #[test]
    fn oversized_replay_event_emits_a_bounded_parser_failing_marker() {
        let (_, bytes) = capture_replay_events(|| {
            emit_replay_event(json!({
                "event": "episode_complete",
                "padding": "x".repeat(MAX_REPLAY_EVENT_BYTES)
            }));
        });
        let row: serde_json::Value = serde_json::from_slice(&bytes).expect("marker JSON");
        assert_eq!(row["event"], "replay_stream_truncated");
        assert_eq!(row["source_event"], "episode_complete");
        assert_eq!(row["reason"], "event_exceeds_bound");
        assert!(bytes.len() < 256);
    }

    struct PlanTransport {
        responses: VecDeque<Vec<u8>>,
    }

    impl ExoTransport for PlanTransport {
        fn exchange(
            &mut self,
            _request: &[u8],
            _max_response_bytes: usize,
            _timeout_millis: u32,
        ) -> Result<Vec<u8>, ExoTransportError> {
            self.responses
                .pop_front()
                .ok_or(ExoTransportError::MalformedResponse)
        }

        fn close(&mut self) -> Result<(), ExoTransportError> {
            Ok(())
        }
    }

    fn plan_input(generation: u64, state_id: &str, hand: serde_json::Value) -> DecisionInput {
        let legal_actions = if generation == 1 {
            json!([
                {"action_id":"play-card","action":{"kind":"play_card","card_id":"card-1","target_id":"enemy-1"}},
                {"action_id":"end-turn","action":{"kind":"end_turn"}}
            ])
        } else {
            json!([
                {"action_id":"end-turn","action":{"kind":"end_turn"}}
            ])
        };
        let observation_value = json!({
            "state_id": state_id,
            "generation": generation,
            "visible_seed": "PLAN-SEED",
            "player": {
                "hp": 80,
                "max_hp": 80,
                "energy": 3,
                "gold": 99,
                "hand": hand,
                "deck": [],
                "discard": [],
                "exhaust": []
            },
            "state": {"state":"combat","turn_index":1,"enemies":[]},
            "legal_actions": legal_actions
        });
        let observation = EpisodeObservation::new(
            state_id,
            generation,
            EpisodeStage::Combat,
            true,
            false,
            true,
            observation_value,
        )
        .expect("plan observation");
        let actions = if generation == 1 {
            vec![
                EpisodeLegalAction::new("play-card", ActionKind::PlayCard)
                    .expect("play-card action"),
                EpisodeLegalAction::new("end-turn", ActionKind::EndTurn).expect("end-turn action"),
            ]
        } else {
            vec![EpisodeLegalAction::new("end-turn", ActionKind::EndTurn).expect("end-turn action")]
        };
        let action_set =
            EpisodeLegalActionSet::new(state_id, generation, actions).expect("plan action set");
        DecisionInput::new(
            ModelExecutionId::new(generation).expect("plan execution"),
            observation,
            action_set,
            "complete the combat",
            Vec::new(),
        )
    }

    #[test]
    fn plan_expansion_records_each_action_with_the_reused_model_execution() {
        const REVISION: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let transport = PlanTransport {
            responses: VecDeque::from([br#"{"decision":"plan","action_ids":["play-card","end-turn"],"rationale":"safe plan"}"#.to_vec()]),
        };
        let config =
            ExoConfig::new(REVISION, 128 * 1024, 8 * 1024, 1_000).expect("plan Exo config");
        let provider = ExoProvider::new(transport, config);
        let mut source = ExoDecisionSource::new(ExoSession::new(provider));
        let first_input = plan_input(
            1,
            "combat-1",
            json!([{"card_id":"card-1","name":"Strike","cost":1,"upgraded":false}]),
        );
        let second_input = plan_input(2, "combat-2", json!([]));
        let mut recorder = DecisionRecorder::new(
            &mut source,
            super::super::super::runtime_v3_telemetry::TelemetryHandle::disabled(),
        );
        let (_, bytes) = capture_replay_events(|| {
            let first = recorder.decide(&first_input).expect("first plan action");
            assert!(
                matches!(first, sts2_harness::Decision::Action { ref action_id, .. } if action_id == "play-card")
            );
            recorder.action_completed(true);
            let second = recorder.decide(&second_input).expect("second plan action");
            assert!(
                matches!(second, sts2_harness::Decision::Action { ref action_id, .. } if action_id == "end-turn")
            );
            recorder.action_completed(true);
        });
        let rows = std::str::from_utf8(&bytes)
            .expect("replay UTF-8")
            .lines()
            .map(serde_json::from_str::<serde_json::Value>)
            .collect::<Result<Vec<_>, _>>()
            .expect("replay rows");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["action_id"], "play-card");
        assert_eq!(rows[0]["model_execution_id"], 1);
        assert_eq!(rows[0]["reused_model_execution"], false);
        assert_eq!(rows[1]["action_id"], "end-turn");
        assert_eq!(rows[1]["model_execution_id"], 1);
        assert_eq!(rows[1]["reused_model_execution"], true);
    }
}
