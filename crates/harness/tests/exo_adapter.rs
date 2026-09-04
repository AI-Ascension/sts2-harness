// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use serde_json::json;
use sts2_harness::{
    Correlation, ExoConfig, ExoProvider, ExoTransport, ExoTransportError, IdempotencyKey,
    ModelExecutionId, ModelRequest, Prompt, ProviderPort,
};

#[derive(Debug)]
struct FakeTransport {
    response: Result<Vec<u8>, ExoTransportError>,
    closed: bool,
}

impl FakeTransport {
    fn new(response: Result<Vec<u8>, ExoTransportError>) -> Self {
        Self {
            response,
            closed: false,
        }
    }
}

impl ExoTransport for FakeTransport {
    fn exchange(
        &mut self,
        _request: &[u8],
        _max_response_bytes: usize,
        _timeout_millis: u32,
    ) -> Result<Vec<u8>, ExoTransportError> {
        self.response.clone()
    }

    fn close(&mut self) -> Result<(), ExoTransportError> {
        self.closed = true;
        Ok(())
    }
}

fn request() -> ModelRequest {
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
        "observation": {
            "state_id": "combat-1",
            "generation": 0,
            "visible_seed": "visible-seed",
            "player": {"hp": 10, "max_hp": 10, "energy": 3, "gold": 0, "hand": [], "deck": [], "discard": [], "exhaust": []},
            "state": {"state": "combat", "turn_index": 1, "enemies": []},
            "legal_actions": [{"action_id": "combat.end-turn", "action": {"kind": "end_turn"}}]
        },
        "state_id": "combat-1",
        "generation": 0,
        "legal_action_ids": ["combat.end-turn"],
        "objective": "survive",
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

fn provider(response: Result<Vec<u8>, ExoTransportError>) -> ExoProvider<FakeTransport> {
    ExoProvider::new(
        FakeTransport::new(response),
        ExoConfig::new(
            "7801005e6a1ab77008a05dbba80e0a2a7a56e35d",
            64 * 1024,
            1024,
            1_000,
        )
        .expect("test Exo configuration is valid"),
    )
}

#[test]
fn unconfigured_or_floating_revision_is_rejected() {
    for revision in [
        "",
        "main",
        "HEAD",
        "latest",
        "REPLACE_WITH_REVIEWED_EXO_REVISION",
        "0000000000000000000000000000000000000000",
        "7801005E6A1AB77008A05DBBA80E0A2A7A56E35D",
    ] {
        assert!(
            ExoConfig::new(revision, 64 * 1024, 1024, 1_000).is_err(),
            "revision {revision:?} must not be accepted as pinned"
        );
    }
    assert!(
        ExoConfig::new(
            "7801005e6a1ab77008a05dbba80e0a2a7a56e35d",
            64 * 1024,
            1024,
            1_000,
        )
        .is_ok()
    );
}

#[test]
fn transport_failures_are_typed_and_do_not_select_an_action() {
    let cases = [
        (ExoTransportError::Unavailable, "exo_unavailable"),
        (ExoTransportError::Timeout, "exo_timeout"),
        (
            ExoTransportError::OversizedResponse,
            "exo_oversized_response",
        ),
        (
            ExoTransportError::MalformedResponse,
            "exo_malformed_response",
        ),
    ];
    for (failure, code) in cases {
        let mut provider = provider(Err(failure));
        let error = provider
            .execute(&request())
            .expect_err("transport failure must be returned");
        assert_eq!(error.code(), code);
    }
}

#[test]
fn malformed_and_oversized_decisions_fail_closed() {
    let mut malformed = provider(Ok(br#"{"decision":"action"}"#.to_vec()));
    assert_eq!(
        malformed
            .execute(&request())
            .expect_err("missing fields must be rejected")
            .code(),
        "exo_malformed_response"
    );

    let oversized = vec![b'{'; 1_025];
    let mut provider = provider(Ok(oversized));
    assert_eq!(
        provider
            .execute(&request())
            .expect_err("oversized response must be rejected")
            .code(),
        "exo_oversized_response"
    );
}

#[test]
fn close_is_explicit_and_repeatable() {
    let mut provider = provider(Ok(
        br#"{"decision":"reobserve","rationale":"refresh"}"#.to_vec()
    ));
    provider.close().expect("transport close is successful");
    provider.close().expect("repeated close is a no-op");
}
