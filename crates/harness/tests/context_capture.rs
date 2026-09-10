// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::sync::{Arc, Mutex};

use serde_json::json;
use sts2_harness::{
    CaptureBoundary, CaptureError, CaptureInput, CapturePort, CaptureRecord, Correlation,
    EpisodeId, ExoConfig, ExoProvider, ExoSession, ExoTransport, ExoTransportError, IdempotencyKey,
    InstanceId, ModelExecutionId, ModelRequest, Prompt, ProviderPort, RunId, TraceId, TrajectoryId,
};

#[derive(Debug)]
struct FakeTransport {
    requests: Vec<Vec<u8>>,
    fail: bool,
}

impl ExoTransport for FakeTransport {
    fn exchange(
        &mut self,
        request: &[u8],
        _max_response_bytes: usize,
        _timeout_millis: u32,
    ) -> Result<Vec<u8>, ExoTransportError> {
        self.requests.push(request.to_vec());
        if self.fail {
            return Err(ExoTransportError::Unavailable);
        }
        Ok(br#"{"decision":"reobserve","rationale":"synthetic"}"#.to_vec())
    }

    fn close(&mut self) -> Result<(), ExoTransportError> {
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct SharedCapture(Arc<Mutex<Vec<CaptureRecord>>>);

impl CapturePort for SharedCapture {
    fn prepared(&mut self, input: CaptureInput<'_>) -> Result<(), CaptureError> {
        self.0.lock().expect("capture lock").push(CaptureRecord {
            snapshot_id: format!("snapshot-{}", input.execution_id),
            execution_id: input.execution_id.to_owned(),
            attempt_id: input.attempt_id.map(str::to_owned),
            boundary: input.boundary,
            state: sts2_harness::TransportState::Prepared,
            observed_bytes: input.bytes.len(),
            sha256: None,
            content: None,
            component_kind: None,
            ordinal: None,
            media_type: None,
        });
        Ok(())
    }

    fn write_completed(&mut self, execution_id: &str) -> Result<(), CaptureError> {
        self.0.lock().expect("capture lock").push(CaptureRecord {
            snapshot_id: format!("snapshot-{execution_id}"),
            execution_id: execution_id.to_owned(),
            attempt_id: None,
            boundary: CaptureBoundary::ProviderRequest,
            state: sts2_harness::TransportState::WriteCompleted,
            observed_bytes: 0,
            sha256: None,
            content: None,
            component_kind: None,
            ordinal: None,
            media_type: None,
        });
        Ok(())
    }

    fn write_failed(&mut self, execution_id: &str, _code: &str) -> Result<(), CaptureError> {
        self.0.lock().expect("capture lock").push(CaptureRecord {
            snapshot_id: format!("snapshot-{execution_id}"),
            execution_id: execution_id.to_owned(),
            attempt_id: None,
            boundary: CaptureBoundary::ProviderRequest,
            state: sts2_harness::TransportState::WriteFailed,
            observed_bytes: 0,
            sha256: None,
            content: None,
            component_kind: None,
            ordinal: None,
            media_type: None,
        });
        Ok(())
    }
}

fn config() -> ExoConfig {
    ExoConfig::new(
        "7801005e6a1ab77008a05dbba80e0a2a7a56e35d",
        64 * 1024,
        8 * 1024,
        1000,
    )
    .expect("config")
}

fn observation() -> serde_json::Value {
    json!({
        "state_id":"combat-1", "generation":0, "visible_seed":"synthetic",
        "player":{"hp":10,"max_hp":10,"energy":3,"gold":1,"hand":[],"deck":[],"discard":[],"exhaust":[]},
        "state":{"state":"combat","turn_index":1,"enemies":[]},
        "legal_actions":[{"action_id":"combat.end-turn","action":{"kind":"end_turn"}}]
    })
}

#[test]
fn direct_exo_session_records_prepared_and_write_without_receipt() {
    let records = Arc::new(Mutex::new(Vec::new()));
    let capture = SharedCapture(Arc::clone(&records));
    let mut session = ExoSession::new(ExoProvider::new(
        FakeTransport {
            requests: Vec::new(),
            fail: false,
        },
        config(),
    ))
    .with_capture(Box::new(capture));
    session
        .decide(
            ModelExecutionId::new(7).expect("id"),
            "combat-1",
            0,
            sts2_harness::SanitizedObservation::new(observation()).expect("observation"),
            vec!["combat.end-turn".to_owned()],
            "synthetic objective",
            Vec::new(),
        )
        .expect("decision");
    let records = records.lock().expect("records");
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].boundary, CaptureBoundary::ExoSessionRequest);
    assert_eq!(records[0].state, sts2_harness::TransportState::Prepared);
    assert_eq!(
        records[1].state,
        sts2_harness::TransportState::WriteCompleted
    );
    assert!(
        records
            .iter()
            .all(|record| record.state != sts2_harness::TransportState::ReceiptReported)
    );
}

#[test]
fn generic_provider_route_records_failure_without_retry() {
    let records = Arc::new(Mutex::new(Vec::new()));
    let execution = ModelExecutionId::new(8).expect("id");
    let correlation = Correlation::for_episode(
        RunId::new(1).expect("id"),
        EpisodeId::new(2).expect("id"),
        TrajectoryId::new(3).expect("id"),
        InstanceId::new(4).expect("id"),
        TraceId::new(5).expect("id"),
    )
    .with_model_execution(execution);
    let prompt = json!({
        "observation": observation(), "state_id":"combat-1", "generation":0,
        "legal_action_ids":["combat.end-turn"], "objective":"synthetic", "hard_constraints":[]
    })
    .to_string();
    let request = ModelRequest::new(
        execution,
        correlation,
        Prompt::new(prompt).expect("prompt"),
        IdempotencyKey::new("capture-test").expect("key"),
    );
    let mut provider = ExoProvider::new(
        FakeTransport {
            requests: Vec::new(),
            fail: true,
        },
        config(),
    )
    .with_capture(Box::new(SharedCapture(Arc::clone(&records))));
    let result = provider.execute(&request);
    assert_eq!(result.expect_err("failure").code(), "exo_unavailable");
    let records = records.lock().expect("records");
    assert_eq!(records.len(), 2);
    assert_eq!(records[1].state, sts2_harness::TransportState::WriteFailed);
}
