// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use sts2_harness::ExoTransport;
use sts2_harness::exo_lifecycle::{AuthorityGuard, InvocationManifest, LifecycleError};

use super::super::super::lifecycle_authority::RuntimeLifecycleFence;
use super::Fixture;

/// The turn ceiling used by the admission tests, which assert admission rather than timing.
///
/// This is not the effect's own budget. The fixture's lifecycle effect is armed from the
/// `ExoConfig` timeout (1000 ms by default), and a caller ceiling below that value can only be
/// reached by a host slow enough to have already spent the effect's budget scheduling the child.
const ADMISSION_TURN_BUDGET_MILLIS: u32 = 30_000;

struct RecordingFence {
    calls: Arc<Mutex<Vec<Option<String>>>>,
    releases: Arc<AtomicUsize>,
}

struct RecordingFenceGuard(Arc<AtomicUsize>);
impl AuthorityGuard for RecordingFenceGuard {}
impl Drop for RecordingFenceGuard {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

impl RuntimeLifecycleFence for RecordingFence {
    fn validate<'a>(
        &'a self,
        _manifest: &InvocationManifest,
        result_digest: Option<&str>,
    ) -> Result<Box<dyn AuthorityGuard + 'a>, LifecycleError> {
        self.calls
            .lock()
            .map_err(|_| LifecycleError::Fenced)?
            .push(result_digest.map(str::to_owned));
        Ok(Box::new(RecordingFenceGuard(self.releases.clone())))
    }
}

#[test]
fn inspected_runtime_admission_executes_one_valid_durable_exchange() {
    let fixture = Fixture::new();
    let request: sts2_harness::ExoDecisionRequest =
        serde_json::from_value(fixture.request.clone()).expect("typed Exo request");
    assert!(
        fixture.authority_state.bind_request(&request).is_ok(),
        "fixture request matches the installed MCP authority"
    );
    let mut transport = fixture.admit().expect("actual lifecycle admission");
    let bytes = serde_json::to_vec(&fixture.request).expect("request");
    // This test's subject is admission, not the turn deadline. The budget below is the turn
    // ceiling the admitted transport clamps against `max_turn_time_millis`; it is sized to the
    // effect's own budget rather than to how fast a loaded host happens to schedule a child, so
    // a slow runner cannot turn an admitted exchange into a reported timeout. Timeout behaviour
    // is asserted deterministically elsewhere, against a peer that overran its ceiling.
    let response = transport.exchange(&bytes, 8192, ADMISSION_TURN_BUDGET_MILLIS);
    assert!(
        response.is_ok(),
        "current observed turn is admitted; response={response:?}, effect_started={}",
        fixture.calls.exists()
    );
    let response = response.expect("validated response");
    let response: Value = serde_json::from_slice(&response).expect("decision response");
    assert_eq!(response["action_id"], "combat.end-turn");
    assert_eq!(
        std::fs::read_to_string(&fixture.calls)
            .expect("effect log")
            .lines()
            .count(),
        1
    );
}

#[test]
fn configured_runtime_fence_is_held_across_send_and_result_consumption() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let releases = Arc::new(AtomicUsize::new(0));
    let fixture = Fixture::new_with_fence(Arc::new(RecordingFence {
        calls: calls.clone(),
        releases: releases.clone(),
    }));
    let mut transport = fixture.admit().expect("actual lifecycle admission");
    let bytes = serde_json::to_vec(&fixture.request).expect("request");
    transport
        .exchange(&bytes, 8192, ADMISSION_TURN_BUDGET_MILLIS)
        .expect("fenced exchange");
    let stored = fixture
        .durable
        .lifecycle_store()
        .borrow()
        .decision("execution-11")
        .expect("durable lifecycle result")
        .result_payload
        .expect("stored response bytes");
    assert_eq!(releases.load(Ordering::SeqCst), 2);
    assert_eq!(
        *calls.lock().expect("recorded fence calls"),
        vec![None, Some(sts2_harness::sha256_hex(stored))]
    );
}

#[test]
fn stale_observation_and_swapped_catalog_fail_before_the_lifecycle_effect() {
    for swapped_catalog in [false, true] {
        let fixture = Fixture::new();
        let mut transport = fixture.admit().expect("actual lifecycle admission");
        let mut request = fixture.request.clone();
        if swapped_catalog {
            request["legal_action_ids"] = json!(["combat.swapped"]);
            request["observation"]["legal_actions"] = json!([
                {"action_id":"combat.swapped","action":{"kind":"end_turn"}}
            ]);
        } else {
            request["state_id"] = json!("combat-stale");
            request["observation"]["state_id"] = json!("combat-stale");
        }
        let bytes = serde_json::to_vec(&request).expect("request");
        // A deadline reached here would still satisfy this assertion, so the call keeps the
        // fixture's own short budget: `is_err()` is the claim under test, and the assertions that
        // follow (`!fixture.calls.exists()`) are what keep a timeout from passing it vacuously.
        assert!(transport.exchange(&bytes, 8192, 1000).is_err());
        assert!(!fixture.calls.exists());
    }
}

#[test]
fn swapped_inspected_executor_refuses_actual_admission_before_effect() {
    let fixture = Fixture::new();
    std::fs::write(&fixture.executor, "#!/bin/sh\nexit 98\n").expect("swap executor bytes");
    assert!(fixture.admit().is_err());
    assert!(!fixture.calls.exists());
}

#[test]
fn authority_revocation_after_send_prevents_result_consumption() {
    let fixture = Fixture::new_with_blocked_effect_timeout(5_000);
    let mut transport = fixture.admit().expect("actual lifecycle admission");
    let authority = fixture.authority_state.clone();
    let calls = fixture.calls.clone();
    let release = fixture.release.clone();
    let invalidator = std::thread::spawn(move || {
        // The workspace runtime test binary starts many process fixtures concurrently. Keep the
        // readiness and process-effect bounds finite while allowing a loaded runner to schedule
        // this child.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !calls.exists() {
            if std::time::Instant::now() >= deadline {
                return Err(String::from("lifecycle process did not start"));
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        authority.invalidate();
        std::fs::write(release, b"release").map_err(|error| error.to_string())
    });
    let bytes = serde_json::to_vec(&fixture.request).expect("request");
    // Revocation is the claim, and a deadline reaching this point would report the same
    // `Err` while still recording an unknown, uncompleted decision. The assertions below name
    // both, so the fixture keeps the budget that makes the race reachable on purpose.
    let response = transport.exchange(&bytes, 8192, 1000);
    invalidator
        .join()
        .expect("authority invalidator thread")
        .expect("process start and revocation");
    assert!(response.is_err());
    assert!(fixture.calls.exists());
    let store = fixture.durable.lifecycle_store();
    let decision = store
        .borrow()
        .decision("execution-11")
        .expect("durable provider decision");
    assert!(decision.unknown);
    assert!(!decision.completed);
}
