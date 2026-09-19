// SPDX-License-Identifier: MIT

//! Operator-line coverage for the supervised children that share the provider transport's
//! diagnostic shape: the lifecycle effect and the lookup supervisor. Refs #352.

#![allow(clippy::expect_used)]

#[cfg(unix)]
use std::cell::RefCell;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::rc::Rc;
#[cfg(unix)]
use std::sync::Arc;
#[cfg(unix)]
use std::time::{Duration, SystemTime};

#[cfg(unix)]
use sts2_harness as harness_api;
#[cfg(unix)]
use sts2_harness::context_memory::MemoryScope;
#[cfg(unix)]
use sts2_harness::exo_lifecycle::{
    ExoLifecycleRuntimeTransport, InvocationManifest, LifecycleError, LifecycleProcessEffect,
};
#[cfg(unix)]
use sts2_harness::exo_lookup_process::ExoLookupProcess;
#[cfg(unix)]
use sts2_harness::exo_lookup_wire::EXO_LOOKUP_FEEDBACK_BYTES;
#[cfg(unix)]
use sts2_harness::game_information::{
    LookupAgentInput, LookupAgentPort, LookupBinding, LookupFeedback,
};
#[cfg(unix)]
use sts2_harness::{
    ActionKind, EXO_SOURCE_REVISION, EpisodeLegalAction, EpisodeLegalActionSet,
    ExecutionCancellation, ExecutionStore, ExoDecisionRequest, ExoProcessConfig, ExoTransport,
};

#[cfg(unix)]
#[path = "support/exo_lifecycle.rs"]
mod fixture;

/// Run one helper in a process of its own and return the helper's own standard error.
///
/// The report is written to the harness's standard error, which a test can only read back from
/// another process. A helper run without its case name is a no-op, so the ordinary test run is
/// unaffected.
#[cfg(unix)]
fn helper_stderr(helper: &str, variable: &str, case: &str) -> String {
    let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
        .args(["--exact", helper, "--nocapture"])
        .env(variable, case)
        .output()
        .expect("child test process");
    assert!(
        output.status.success(),
        "child helper failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Drive the lookup supervisor with one bridge that cannot answer, and let the harness report why.
#[cfg(unix)]
#[test]
fn lookup_supervisor_failure_child_helper() {
    let Some(case) = std::env::var_os("STS2_TEST_EXO_DIAGNOSTIC_LOOKUP") else {
        return;
    };
    let case = case.to_str().expect("case name");
    let config = match case {
        "missing" => {
            ExoProcessConfig::new("/nonexistent/lookup-bridge", Vec::new(), None, Vec::new())
        }
        "message" => ExoProcessConfig::new(
            "/bin/sh",
            vec![
                String::from("-c"),
                String::from("printf '%s\\n' 'lookup bridge has no interpreter' >&2; exit 7"),
            ],
            None,
            Vec::new(),
        ),
        _ => ExoProcessConfig::new(
            "/bin/sh",
            vec![
                String::from("-c"),
                String::from("printf '%s\\n' 'lookup bridge is stuck' >&2; sleep 30"),
            ],
            None,
            Vec::new(),
        ),
    }
    .expect("bridge configuration is valid");
    let timeout = if case == "stuck" {
        Duration::from_millis(300)
    } else {
        Duration::from_secs(5)
    };
    let mut agent = ExoLookupProcess::new(
        config,
        String::from("request-1"),
        String::from("turn-1"),
        lookup_request(),
        timeout,
    )
    .expect("bounded supervisor configuration");
    let binding = lookup_binding();
    let legal = legal_actions();
    let feedback = LookupFeedback::Start;
    let input = LookupAgentInput {
        binding: &binding,
        legal_actions: &legal,
        feedback: &feedback,
        remaining_turns: 3,
        optional_byte_budget: EXO_LOOKUP_FEEDBACK_BYTES,
    };
    assert!(
        agent.next_turn(input).is_err(),
        "a bridge that cannot answer produced a turn"
    );
}

/// Drive the one-shot lifecycle effect with one bridge that cannot answer.
#[cfg(unix)]
#[test]
fn lifecycle_effect_failure_child_helper() {
    let Some(case) = std::env::var_os("STS2_TEST_EXO_DIAGNOSTIC_LIFECYCLE") else {
        return;
    };
    let case = case.to_str().expect("case name");
    let mut fixture = fixture::Fixture::new();
    let script = fixture.root.join("bridge");
    let body = if case == "message" {
        String::from(
            "#!/bin/sh\nprintf '%s\\n' 'lifecycle bridge has no interpreter' >&2\nexit 7\n",
        )
    } else {
        String::from("#!/bin/sh\nprintf '%s\\n' 'lifecycle bridge is stuck' >&2\nsleep 30\n")
    };
    std::fs::write(&script, body).expect("bridge script");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700))
        .expect("executable bridge");
    let effect = LifecycleProcessEffect::new(
        ExoProcessConfig::new(
            script.to_string_lossy(),
            vec![
                String::from("--run-v2"),
                String::from("/tmp/exo-lifecycle-config"),
                String::from("digest"),
            ],
            None,
            Vec::new(),
        )
        .expect("bridge configuration is valid"),
        ExecutionCancellation::default(),
        8 * 1024,
        if case == "message" { 1_000 } else { 300 },
        3,
    )
    .expect("bounded effect configuration");
    let replacement = ExecutionStore::open_in_memory().expect("store");
    let store = Rc::new(RefCell::new(std::mem::replace(
        &mut fixture.store,
        replacement,
    )));
    let manifest = fixture.manifest.clone();
    let mut transport = ExoLifecycleRuntimeTransport::new(
        fixture.owner(),
        store,
        fixture.fingerprint.clone(),
        effect,
        3_600,
        Arc::new(SystemTime::now),
        move |_: &str,
              _: &str,
              _: &ExoDecisionRequest|
              -> Result<InvocationManifest, LifecycleError> { Ok(manifest.clone()) },
    )
    .expect("lifecycle runtime transport");
    assert!(
        transport.exchange(&fixture.input, 8 * 1024, 1_000).is_err(),
        "a bridge that cannot answer produced a decision"
    );
}

/// A lifecycle effect that cannot start names its own message, because the effect used to send the
/// child's standard error to the null device and report the failure as an unavailable peer.
#[cfg(unix)]
#[test]
fn a_lifecycle_effect_that_cannot_start_reports_its_exit_status_and_message() {
    let stderr = helper_stderr(
        "lifecycle_effect_failure_child_helper",
        "STS2_TEST_EXO_DIAGNOSTIC_LIFECYCLE",
        "message",
    );
    assert!(
        stderr.contains("lifecycle effect failed: exit status: 7"),
        "no operator line naming the exit status: {stderr}"
    );
    assert!(
        stderr.contains("lifecycle bridge has no interpreter"),
        "no operator line carrying the child's message: {stderr}"
    );
}

/// An effect the deadline stopped is still reported when it left something on the stream, because
/// that is the only evidence about why it stalled.
#[cfg(unix)]
#[test]
fn a_stalled_lifecycle_effect_is_reported_before_its_child_exits() {
    let stderr = helper_stderr(
        "lifecycle_effect_failure_child_helper",
        "STS2_TEST_EXO_DIAGNOSTIC_LIFECYCLE",
        "stuck",
    );
    assert!(
        stderr.contains("lifecycle effect failed before its child exited"),
        "no operator line naming a child that never answered: {stderr}"
    );
    assert!(
        stderr.contains("lifecycle bridge is stuck"),
        "no operator line carrying the child's message: {stderr}"
    );
}

/// A duplex supervisor whose bridge cannot start names the path it could not start, rather than
/// leaving the operator with the same `Transport` error a peer that was down produces.
#[cfg(unix)]
#[test]
fn a_lookup_supervisor_that_cannot_start_names_the_path_it_could_not_start() {
    let stderr = helper_stderr(
        "lookup_supervisor_failure_child_helper",
        "STS2_TEST_EXO_DIAGNOSTIC_LOOKUP",
        "missing",
    );
    assert!(
        stderr.contains("lookup supervisor could not start: "),
        "no operator line naming the failed start: {stderr}"
    );
    assert!(
        stderr.contains("/nonexistent/lookup-bridge"),
        "no operator line carrying the path: {stderr}"
    );
}

/// A lookup bridge that exits by itself is reported with its status and its own message.
#[cfg(unix)]
#[test]
fn a_lookup_bridge_that_exits_reports_its_exit_status_and_message() {
    let stderr = helper_stderr(
        "lookup_supervisor_failure_child_helper",
        "STS2_TEST_EXO_DIAGNOSTIC_LOOKUP",
        "message",
    );
    assert!(
        stderr.contains("lookup supervisor failed: exit status: 7"),
        "no operator line naming the exit status: {stderr}"
    );
    assert!(
        stderr.contains("lookup bridge has no interpreter"),
        "no operator line carrying the child's message: {stderr}"
    );
}

/// The duplex supervisor drains its child's standard error while the child runs, so a bridge that
/// wrote to it and never answered still explains itself when the deadline stops the turn.
#[cfg(unix)]
#[test]
fn a_lookup_bridge_that_never_answers_is_reported_before_its_child_exits() {
    let stderr = helper_stderr(
        "lookup_supervisor_failure_child_helper",
        "STS2_TEST_EXO_DIAGNOSTIC_LOOKUP",
        "stuck",
    );
    assert!(
        stderr.contains("lookup supervisor failed before its child exited"),
        "no operator line naming a child that never answered: {stderr}"
    );
    assert!(
        stderr.contains("lookup bridge is stuck"),
        "no operator line carrying the child's message: {stderr}"
    );
}

/// The discarded stream was the defect, so the guard reads each supervisor's source: restoring the
/// null device fails here instead of silently on the next native launch.
#[test]
fn every_supervised_child_captures_its_standard_error() {
    for relative in [
        "src/exo_process.rs",
        "src/exo_lifecycle/process_effect.rs",
        "src/exo_lookup_process_supervisor.rs",
    ] {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(relative),
        )
        .expect("supervisor source");
        assert!(
            !source.contains("Stdio::null()"),
            "{relative} discards the child's standard error again"
        );
        assert!(
            source.contains(".stderr(Stdio::piped())"),
            "{relative} no longer captures the child's standard error"
        );
    }
    let effect = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/exo_lifecycle/process_effect.rs"),
    )
    .expect("effect source");
    assert!(
        effect.contains("report_stopped_child_failure(LIFECYCLE_EFFECT, &mut child, own_exit)"),
        "a captured standard error is no longer reported"
    );
}

/// The owner-supplied binding the supervisor expects, built from the public wire shape.
#[cfg(unix)]
fn lookup_binding() -> LookupBinding {
    LookupBinding {
        scope: MemoryScope::new("project-1", "run-1", "episode-1", "agent-1"),
        game_profile: String::from("sts2"),
        content_manifest_id: String::from("manifest-1"),
        locale: String::from("en"),
        authority_epoch: 1,
        snapshot: None,
    }
}

/// The legal set `lookup_request` names, so the supervisor is actually asked for a turn.
#[cfg(unix)]
fn legal_actions() -> EpisodeLegalActionSet {
    EpisodeLegalActionSet::new(
        "state-42",
        42,
        vec![
            EpisodeLegalAction::new("play:card-17", ActionKind::PlayCard)
                .expect("legal action identity"),
        ],
    )
    .expect("legal action set")
}

/// One bounded decision request shape, carrying the identity the legal set above names.
#[cfg(unix)]
fn lookup_request() -> serde_json::Value {
    serde_json::json!({
        "schema": "sts2.exo-decision-v1",
        "provider_revision": EXO_SOURCE_REVISION,
        "model_execution_id": "execution-1",
        "state_id": "state-42",
        "generation": 42,
        "observation": {
            "state_id": "state-42",
            "generation": 42,
            "visible_seed": null,
            "player": {
                "hp": 10, "max_hp": 10, "energy": 3, "gold": 0,
                "hand": [], "deck": [], "discard": [], "exhaust": []
            },
            "state": {"state": "combat", "turn_index": 1, "enemies": []},
            "legal_actions": [{
                "action_id": "play:card-17",
                "action": {"kind": "play_card", "card_id": "card-17", "target_id": null}
            }]
        },
        "legal_action_ids": ["play:card-17"],
        "objective": "survive",
        "hard_constraints": [],
        "max_response_bytes": 8192
    })
}
