// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::collections::VecDeque;

use serde_json::{Value, json};
use sts2_harness::{
    Correlation, Decision, ExoConfig, ExoProvider, ExoSession, ExoTransport, ExoTransportError,
    IdempotencyKey, ModelExecutionId, ModelRequest, Prompt, ProviderPort, SanitizedObservation,
};

const REVISION: &str = "7801005e6a1ab77008a05dbba80e0a2a7a56e35d";
const HOST_SEED: &str = "seed-shown-by-host";

#[derive(Debug)]
struct RecordingTransport {
    requests: Vec<Vec<u8>>,
    responses: VecDeque<Vec<u8>>,
}

impl RecordingTransport {
    fn new() -> Self {
        Self {
            requests: Vec::new(),
            responses: VecDeque::from([
                br#"{"decision":"reobserve","rationale":"refresh"}"#.to_vec()
            ]),
        }
    }
}

impl ExoTransport for RecordingTransport {
    fn exchange(
        &mut self,
        request: &[u8],
        _max_response_bytes: usize,
        _timeout_millis: u32,
    ) -> Result<Vec<u8>, ExoTransportError> {
        self.requests.push(request.to_vec());
        Ok(self
            .responses
            .pop_front()
            .unwrap_or_else(|| br#"{"decision":"reobserve","rationale":"refresh"}"#.to_vec()))
    }

    fn close(&mut self) -> Result<(), ExoTransportError> {
        Ok(())
    }
}

fn observation() -> serde_json::Value {
    json!({
        "state_id": "map-1",
        "generation": 4,
        "visible_seed": HOST_SEED,
        "player": {"hp": 10, "max_hp": 10, "energy": 3, "gold": 20, "hand": [], "deck": [], "discard": [], "exhaust": []},
        "state": {"state": "map", "node_id": "node-1", "options": ["node-2"]},
        "legal_actions": [{"action_id": "map.select", "action": {"kind": "select_map_node", "node_id": "node-2"}}]
    })
}

fn config() -> ExoConfig {
    ExoConfig::new(REVISION, 64 * 1024, 8 * 1024, 1_000).expect("configuration is valid")
}

/// Runs one session decision and returns the exact bytes the transport received.
fn session_request(config: ExoConfig) -> Value {
    let mut session = ExoSession::new(ExoProvider::new(RecordingTransport::new(), config));
    let decision = session
        .decide(
            ModelExecutionId::new(12).expect("execution ID is nonzero"),
            "map-1",
            4,
            SanitizedObservation::new(observation()).expect("observation is fair play"),
            vec!["map.select".to_owned()],
            "advance",
            vec!["Use only the current legal action set".to_owned()],
        )
        .expect("provider response is valid");
    assert_eq!(
        decision,
        Decision::Reobserve {
            rationale: "refresh".to_owned()
        }
    );
    let transport = session.into_transport();
    assert_eq!(transport.requests.len(), 1);
    serde_json::from_slice(&transport.requests[0]).expect("request is JSON")
}

fn assert_seed_absent(request: &Value) {
    assert!(
        request["observation"].get("visible_seed").is_none(),
        "visible_seed key must be absent by default"
    );
    let encoded = serde_json::to_string(request).expect("request can be inspected");
    assert!(!encoded.contains(HOST_SEED), "request leaked the host seed");
    assert!(!encoded.contains("visible_seed"));
    for forbidden in [
        "raw_memory",
        "future_rng",
        "unrevealed",
        "host_object",
        "credential",
        "private_prompt",
        "screen_coordinate",
        "input_event",
    ] {
        assert!(!encoded.contains(forbidden), "request leaked {forbidden}");
    }
}

#[test]
fn exo_request_omits_visible_seed_by_default() {
    let config = config();
    assert!(!config.forward_visible_seed, "gate must default to off");
    let request = session_request(config);
    assert_eq!(request["provider_revision"], REVISION);
    assert_eq!(request["legal_action_ids"], json!(["map.select"]));
    assert_eq!(request["observation"]["state_id"], "map-1");
    assert_eq!(request["observation"]["generation"], 4);
    assert_seed_absent(&request);
}

#[test]
fn visible_seed_is_forwarded_only_with_the_explicit_gate() {
    let request = session_request(config().with_visible_seed_forwarding(true));
    assert_eq!(request["observation"]["visible_seed"], HOST_SEED);

    let request = session_request(config().with_visible_seed_forwarding(false));
    assert_seed_absent(&request);
}

fn model_request(observation: Value) -> ModelRequest {
    let execution_id = ModelExecutionId::new(11).expect("execution ID is nonzero");
    let correlation = Correlation::for_episode(
        sts2_harness::RunId::new(1).expect("run ID is nonzero"),
        sts2_harness::EpisodeId::new(2).expect("episode ID is nonzero"),
        sts2_harness::TrajectoryId::new(3).expect("trajectory ID is nonzero"),
        sts2_harness::InstanceId::new(4).expect("instance ID is nonzero"),
        sts2_harness::TraceId::new(5).expect("trace ID is nonzero"),
    )
    .with_model_execution(execution_id);
    let prompt = json!({
        "observation": observation,
        "state_id": "map-1",
        "generation": 4,
        "legal_action_ids": ["map.select"],
        "objective": "advance",
        "hard_constraints": []
    })
    .to_string();
    ModelRequest::new(
        execution_id,
        correlation,
        Prompt::new(prompt).expect("structured prompt is valid"),
        IdempotencyKey::new("exo-request-11").expect("idempotency key is valid"),
    )
}

fn provider_request(config: ExoConfig) -> Value {
    let mut provider = ExoProvider::new(RecordingTransport::new(), config);
    provider
        .execute(&model_request(observation()))
        .expect("provider response is valid");
    let transport = provider.into_transport();
    assert_eq!(transport.requests.len(), 1);
    serde_json::from_slice(&transport.requests[0]).expect("request is JSON")
}

#[test]
fn provider_port_prompt_path_applies_the_same_seed_gate() {
    assert_seed_absent(&provider_request(config()));
    let request = provider_request(config().with_visible_seed_forwarding(true));
    assert_eq!(request["observation"]["visible_seed"], HOST_SEED);
}

#[test]
fn host_seed_without_a_value_is_still_redacted_and_never_synthesized() {
    let mut value = observation();
    value["visible_seed"] = json!(null);
    let redacted = SanitizedObservation::new(value)
        .expect("null seed is fair play")
        .without_visible_seed();
    assert!(!redacted.has_visible_seed());
    assert!(
        SanitizedObservation::new(redacted.as_value().clone()).is_ok(),
        "redacted projection remains a valid fair-play observation"
    );
}
