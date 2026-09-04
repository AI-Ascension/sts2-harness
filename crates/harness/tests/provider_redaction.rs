// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::collections::VecDeque;

use serde_json::{Value, json};
use sts2_harness::{
    Decision, ExoConfig, ExoProvider, ExoSession, ExoTransport, ExoTransportError,
    ModelExecutionId, SanitizedObservation,
};

#[derive(Debug)]
struct RecordingTransport {
    requests: Vec<Vec<u8>>,
    responses: VecDeque<Vec<u8>>,
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
        "visible_seed": "seed-shown-by-host",
        "player": {"hp": 10, "max_hp": 10, "energy": 3, "gold": 20, "hand": [], "deck": [], "discard": [], "exhaust": []},
        "state": {"state": "map", "node_id": "node-1", "options": ["node-2"]},
        "legal_actions": [{"action_id": "map.select", "action": {"kind": "select_map_node", "node_id": "node-2"}}]
    })
}

#[test]
fn exo_request_contains_only_sanitized_fair_play_fields() {
    let transport = RecordingTransport {
        requests: Vec::new(),
        responses: VecDeque::from([br#"{"decision":"reobserve","rationale":"refresh"}"#.to_vec()]),
    };
    let config = ExoConfig::new("exo-pinned-revision", 64 * 1024, 8 * 1024, 1_000)
        .expect("configuration is valid");
    let mut session = ExoSession::new(ExoProvider::new(transport, config));
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
    let request: Value = serde_json::from_slice(&transport.requests[0]).expect("request is JSON");
    assert_eq!(request["provider_revision"], "exo-pinned-revision");
    assert_eq!(request["observation"]["visible_seed"], "seed-shown-by-host");
    assert_eq!(request["legal_action_ids"], json!(["map.select"]));
    let encoded = serde_json::to_string(&request).expect("request can be inspected");
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
