// SPDX-License-Identifier: MIT

use std::collections::VecDeque;

use super::{
        DecisionRecorder, GameOutcome, MAX_REPLAY_EVENT_BYTES, capture_replay_events,
        dispatch_status_name, emit_replay_event, encode_replay_event, game_outcome,
        replay_failure_code, complete_observation,
};
use serde_json::json;
use sts2_harness::{
        ActionKind, DecisionInput, DecisionSource, EpisodeLegalAction, EpisodeLegalActionSet,
        EpisodeObservation, EpisodeRunnerError, EpisodeStage, ExoConfig, ExoDecisionSource,
        ExoProvider, ExoSession, ExoTransport, ExoTransportError, ModelExecutionId, PortError,
    };
use super::super::super::runtime_v3_telemetry::TelemetryHandle;

    #[test]
    fn terminal_stage_outcome_survives_independent_cleanup_status() {
        assert_eq!(game_outcome(EpisodeStage::Victory), GameOutcome::Success);
        assert_eq!(game_outcome(EpisodeStage::Reward), GameOutcome::Unavailable);
        assert_eq!(game_outcome(EpisodeStage::Defeat), GameOutcome::Failure);
        assert_eq!(game_outcome(EpisodeStage::Combat), GameOutcome::Unavailable);
    }

    #[test]
    fn nonterminal_reward_is_not_reported_as_terminal() {
        let reward = EpisodeObservation::new(
            "reward-1",
            1,
            EpisodeStage::Reward,
            false,
            false,
            true,
            json!({
                "state_id": "reward-1",
                "generation": 1,
                "visible_seed": "seed",
                "player": {"hp": 80, "max_hp": 80, "energy": 0, "gold": 0,
                    "hand": [], "deck": [], "discard": [], "exhaust": []},
                "state": {"state": "reward", "options": []},
                "legal_actions": []
            }),
        )
        .expect("reward observation");
        let telemetry = TelemetryHandle::disabled();

        assert!(!complete_observation(&reward, &telemetry));
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
