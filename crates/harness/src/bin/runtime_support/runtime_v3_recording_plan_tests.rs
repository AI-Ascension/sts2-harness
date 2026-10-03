// SPDX-License-Identifier: MIT

// End-to-end tests for how a plan the host undercuts is recorded.
//
// A nested module rather than a sibling file so it can drive the same `PlanTransport` and
// `plan_input` scaffolding the other recording tests use, instead of copying it: one scripted
// transport, two shapes of run to tell apart.

use super::*;

#[test]
fn a_plan_the_host_undercuts_falls_back_to_a_fresh_provider_decision() {
    const REVISION: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    // Two responses, so the second decision can only be answered by a second round trip. If the
    // undercut plan were still believed, the transport would be asked for nothing and the
    // second call would fail instead.
    let transport = PlanTransport {
        responses: VecDeque::from([
            br#"{"decision":"plan","action_ids":["play-card","end-turn"],"rationale":"safe plan"}"#.to_vec(),
            br#"{"decision":"action","action_id":"end-turn","rationale":"re-decided"}"#.to_vec(),
        ]),
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
    let mut recorder = DecisionRecorder::new(
        &mut source,
        super::super::super::super::runtime_v3_telemetry::TelemetryHandle::disabled(),
    );
    let (_, bytes) = capture_replay_events(|| {
        let first = recorder.decide(&first_input).expect("first plan action");
        assert!(
            matches!(first, sts2_harness::Decision::Action { ref action_id, .. } if action_id == "play-card")
        );
        recorder.action_completed(true);

        // The host now offers only `end-turn` and the player has taken damage, so the plan's
        // second step was predicted from a state that did not happen. The plan must end and a
        // fresh decision must be taken: the outcome here is identical to continuing, which is
        // exactly why only the provenance of the row can show the plan was dropped.
        let undercut = combat_with_damage(2, "combat-2");
        let second = recorder.decide(&undercut).expect("fresh decision");
        assert!(
            matches!(second, sts2_harness::Decision::Action { ref action_id, .. } if action_id == "end-turn"),
            "a fresh decision is taken after the plan is dropped"
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
    assert_eq!(rows[0]["reused_model_execution"], false);
    // The second row is a newly decided action, not a reused plan execution: the dispatch is
    // the same, so provenance is the only thing distinguishing "the plan was right" from
    // "the plan was abandoned and we asked again".
    assert_eq!(
        rows[1]["reused_model_execution"], false,
        "an undercut plan must be recorded as a fresh decision, not a reused plan step"
    );
    assert_eq!(rows[1]["model_execution_id"], 2);
}

/// A post-plan state that the plan could not have predicted: the player has taken damage.
fn combat_with_damage(generation: u64, state_id: &str) -> DecisionInput {
    let observation = EpisodeObservation::new(
        state_id,
        generation,
        EpisodeStage::Combat,
        true,
        false,
        true,
        json!({
            "state_id": state_id,
            "generation": generation,
            "visible_seed": "PLAN-SEED",
            "player": {
                "hp": 12,
                "max_hp": 80,
                "energy": 3,
                "gold": 99,
                "hand": [],
                "deck": [],
                "discard": [],
                "exhaust": []
            },
            "state": {"state":"combat","turn_index":1,"enemies":[]},
            "legal_actions": [
                {"action_id":"end-turn","action":{"kind":"end_turn"}}
            ]
        }),
    )
    .expect("undercut observation");
    let action_set = EpisodeLegalActionSet::new(
        state_id,
        generation,
        vec![EpisodeLegalAction::new("end-turn", ActionKind::EndTurn).expect("end-turn")],
    )
    .expect("undercut action set");
    DecisionInput::new(
        ModelExecutionId::new(generation).expect("undercut execution"),
        observation,
        action_set,
        "complete the combat",
        Vec::new(),
    )
}
