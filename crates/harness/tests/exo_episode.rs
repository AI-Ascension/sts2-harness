// SPDX-License-Identifier: MIT

use std::collections::VecDeque;

use serde_json::{Value, json};
use sts2_harness::{
    ActionKind, Decision, DecisionInput, EpisodeLegalAction,
    EpisodeLegalActionSet, EpisodeObservation, EpisodeStage, ExoConfig, ExoDecisionSource,
    ExoError, ExoProvider, ExoSession, ExoTransport, ExoTransportError, ModelExecutionId,
    PolicyRouter, SanitizedObservation, parse_decision,
};

#[derive(Debug)]
struct FakeExo {
    responses: VecDeque<Result<Vec<u8>, ExoTransportError>>,
    requests: Vec<Vec<u8>>,
    timeouts: Vec<u32>,
    close_calls: usize,
}

impl FakeExo {
    fn reply(response: &str) -> Self {
        Self {
            responses: VecDeque::from([Ok(response.as_bytes().to_vec())]),
            requests: Vec::new(),
            timeouts: Vec::new(),
            close_calls: 0,
        }
    }

    fn failure(error: ExoTransportError) -> Self {
        Self {
            responses: VecDeque::from([Err(error)]),
            requests: Vec::new(),
            timeouts: Vec::new(),
            close_calls: 0,
        }
    }
}

impl ExoTransport for FakeExo {
    fn exchange(
        &mut self,
        request: &[u8],
        _max_response_bytes: usize,
        timeout_millis: u32,
    ) -> Result<Vec<u8>, ExoTransportError> {
        self.requests.push(request.to_vec());
        self.timeouts.push(timeout_millis);
        self.responses
            .pop_front()
            .unwrap_or_else(|| Ok(br#"{"decision":"reobserve","rationale":"fresh state"}"#.to_vec()))
    }

    fn close(&mut self) -> Result<(), ExoTransportError> {
        self.close_calls += 1;
        Ok(())
    }
}

fn observation_value(generation: u64) -> Value {
    json!({
        "state_id": "combat-1",
        "generation": generation,
        "visible_seed": "visible-seed-only",
        "player": {
            "hp": 50,
            "max_hp": 50,
            "energy": 3,
            "gold": 99,
            "hand": [],
            "deck": [],
            "discard": [],
            "exhaust": []
        },
        "state": {"state": "combat", "turn_index": 1, "enemies": []},
        "legal_actions": [
            {"action_id": "combat.end-turn", "action": {"kind": "end_turn"}}
        ]
    })
}

fn observation(generation: u64) -> EpisodeObservation {
    EpisodeObservation::new(
        "combat-1",
        generation,
        EpisodeStage::Combat,
        true,
        false,
        true,
        observation_value(generation),
    )
    .expect("test observation is valid")
}

fn legal_actions(generation: u64) -> EpisodeLegalActionSet {
    EpisodeLegalActionSet::new(
        "combat-1",
        generation,
        vec![EpisodeLegalAction::new("combat.end-turn", ActionKind::EndTurn)
            .expect("test action is valid")],
    )
    .expect("test catalog is valid")
}

fn config() -> ExoConfig {
    ExoConfig::new("exo-revision-2026-09-04", 64 * 1024, 8 * 1024, 2_000)
        .expect("test Exo config is valid")
}

#[test]
fn firewall_rejects_privileged_and_unknown_projection_fields() {
    let mut privileged = observation_value(0);
    privileged["raw_memory"] = json!("forbidden");
    assert!(matches!(
        SanitizedObservation::new(privileged),
        Err(sts2_harness::SandboxError::PrivilegedField)
    ));

    let mut unknown = observation_value(0);
    unknown["screen_text"] = json!("not part of the contract");
    assert!(matches!(
        SanitizedObservation::new(unknown),
        Err(sts2_harness::SandboxError::UnknownField)
    ));
}

#[test]
fn episode_rejects_projection_identity_mismatch() {
    let result = EpisodeObservation::new(
        "combat-1",
        2,
        EpisodeStage::Combat,
        true,
        false,
        true,
        observation_value(1),
    );
    assert!(matches!(
        result,
        Err(sts2_harness::ObservationError::ProjectionMismatch)
    ));
}

#[test]
fn exo_decision_is_bounded_to_current_host_catalog() {
    let config = config();
    let transport = FakeExo::reply(
        r#"{"decision":"action","action_id":"combat.end-turn","rationale":"end the turn","confidence":91}"#,
    );
    let provider = ExoProvider::new(transport, config);
    let session = ExoSession::new(provider);
    let mut source = ExoDecisionSource::new(session);
    let input = DecisionInput::new(
        ModelExecutionId::new(7).expect("nonzero execution ID"),
        observation(0),
        legal_actions(0),
        "survive",
        vec!["use only the current legal action IDs".to_owned()],
    );

    let choice = PolicyRouter::choose(&mut source, &input).expect("provider decision is valid");
    assert!(matches!(
        choice,
        sts2_harness::PolicyChoice::Action {
            action_id,
            confidence: Some(91),
            ..
        } if action_id == "combat.end-turn"
    ));
}

#[test]
fn exo_request_carries_pinned_revision_and_timeout() {
    let transport = FakeExo::reply(r#"{"decision":"reobserve","rationale":"refresh"}"#);
    let mut session = ExoSession::new(ExoProvider::new(transport, config()));
    let decision = session
        .decide(
            ModelExecutionId::new(8).expect("nonzero execution ID"),
            "combat-1",
            0,
            SanitizedObservation::new(observation_value(0)).expect("valid projection"),
            vec!["combat.end-turn".to_owned()],
            "survive",
            Vec::new(),
        )
        .expect("transport response is valid");
    assert_eq!(decision, Decision::Reobserve { rationale: "refresh".to_owned() });

    let transport = session.into_transport();
    let request = serde_json::from_slice::<Value>(&transport.requests[0])
        .expect("request is JSON");
    assert_eq!(request["provider_revision"], "exo-revision-2026-09-04");
    assert_eq!(transport.timeouts, vec![2_000]);
}

#[test]
fn strict_decision_parser_rejects_hidden_reasoning_fields_and_illegal_actions() {
    let hidden = parse_decision(
        br#"{"decision":"action","action_id":"combat.end-turn","rationale":"ok","thoughts":"private"}"#,
    );
    assert!(matches!(hidden, Err(sts2_harness::DecisionError::UnknownField)));

    let decision = parse_decision(
        br#"{"decision":"action","action_id":"combat.other","rationale":"ok"}"#,
    )
    .expect("decision syntax is valid");
    assert!(matches!(
        ExoSession::<FakeExo>::bind_action(decision, &["combat.end-turn".to_owned()]),
        Err(sts2_harness::DecisionError::IllegalAction)
    ));
}

#[test]
fn unavailable_exo_has_no_heuristic_fallback() {
    let mut session = ExoSession::new(ExoProvider::new(
        FakeExo::failure(ExoTransportError::Unavailable),
        config(),
    ));
    let result = session.decide(
        ModelExecutionId::new(9).expect("nonzero execution ID"),
        "combat-1",
        0,
        SanitizedObservation::new(observation_value(0)).expect("valid projection"),
        vec!["combat.end-turn".to_owned()],
        "survive",
        Vec::new(),
    );
    assert_eq!(result, Err(ExoError::Unavailable));
}
