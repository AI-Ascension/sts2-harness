// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use super::*;

#[test]
fn stalled_ordinary_handlers_leave_capacity_for_original_request_recovery() {
    let mut fixture = Fixture::start_with_budget(false, 2500);
    let address = fixture.address;
    let mut requests = Vec::new();
    for index in 0..3 {
        requests.push(thread::spawn(move || {
            ManagementClient::new(address, TOKEN)
                .expect("client")
                .request_json(
                    "POST",
                    "/v1/workflow-runs",
                    Some(&body(&format!("stall-capacity-{index}"))),
                )
                .expect("eventual timeout response")
        }));
    }
    fixture.wait_records("opens", 3);
    let held = fixture.process("backend", 0);
    let overflow = fixture.request("POST", "/v1/workflow-runs", Some(&body("must-not-open")));
    assert_eq!(overflow.status, 503);
    assert_eq!(value(overflow)["error"]["class"], "unavailable");
    let before = Instant::now();
    let pending = fixture.request("GET", "/v1/workflow-submissions/stall-capacity-0", None);
    assert_eq!(pending.status, 200);
    assert_eq!(value(pending)["state"], "pending");
    assert!(
        before.elapsed() < Duration::from_secs(1),
        "recovery must not wait for retirement"
    );
    assert_eq!(service::recorded(&fixture.root, "backend").len(), 1);
    assert_eq!(service::recorded(&fixture.root, "opens").len(), 3);
    for request in requests {
        let response = request.join().expect("client thread");
        assert_eq!(response.status, 503);
        assert_eq!(value(response)["error"]["class"], "write_state_unknown");
    }
    assert_exited(&held);
    fixture.stop();
}

#[test]
fn shutdown_during_a_stall_reaps_helper_backend_and_escaped_child() {
    let mut fixture = Fixture::start_with_budget(false, 5000);
    let address = fixture.address;
    let request = thread::spawn(move || {
        ManagementClient::new(address, TOKEN)
            .expect("client")
            .request_json("POST", "/v1/workflow-runs", Some(&body("stall-shutdown")))
            .expect("shutdown response")
    });
    fixture.wait_records("descendant", 1);
    let backend = fixture.process("backend", 0);
    let descendant = fixture.process("descendant", 0);
    let supervisor = fixture.process("supervisor", 0);
    let before = Instant::now();
    fixture.stop();
    assert!(before.elapsed() < Duration::from_secs(4));
    let response = request.join().expect("client thread");
    assert_eq!(response.status, 503);
    assert_eq!(value(response)["error"]["class"], "write_state_unknown");
    for identity in [&backend, &descendant, &supervisor] {
        assert_exited(identity);
    }
}

#[test]
fn a_pre_reservation_stall_keeps_the_claim_without_fabricating_a_run_or_retrying() {
    let mut fixture = Fixture::start(false);
    let mut definition = service::live::definition(false);
    definition["annotations"]["summary"] = json!("owned test stall");
    serde_json::from_value::<crate::workflow::WorkflowDefinition>(definition.clone())
        .expect("valid fixture must reach the injected definition port");
    let request = service::live::request("claim-before-reservation", definition);
    let encoded = serde_json::to_vec(&request).expect("request");
    let response = fixture.request("POST", "/v1/workflow-runs", Some(&encoded));
    assert_eq!(
        response.status,
        503,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    assert_eq!(value(response)["error"]["class"], "write_state_unknown");
    let pending = value(fixture.request(
        "GET",
        "/v1/workflow-submissions/claim-before-reservation",
        None,
    ));
    assert_eq!(pending["state"], "pending");
    assert_eq!(pending["workflow_run_id"], Value::Null);
    assert_eq!(
        fixture
            .request("POST", "/v1/workflow-runs", Some(&encoded))
            .status,
        503
    );
    assert_eq!(
        service::recorded(&fixture.root, "definition-stall").len(),
        1
    );
    assert!(service::recorded(&fixture.root, "opens").is_empty());
    fixture.stop();
}

#[test]
fn blocked_stderr_is_inside_the_backend_budget_and_diagnostic_loss_is_visible() {
    let mut fixture = Fixture::start(true);
    let backend = fixture.process("backend", 0);
    // The frontend's startup marker is queued to the same inherited blocked sink as every
    // lifecycle marker. The helper must retire that generation without waiting on stderr.
    await_exited(&backend);
    assert_eq!(fixture.request("GET", "/v1/health", None).status, 200);
    let lost = fixture.diagnostic_loss();
    assert!(
        lost > 0,
        "dropped diagnostics must not masquerade as an empty successful log"
    );
    fixture.stop();
}
