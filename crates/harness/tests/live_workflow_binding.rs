// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::sync::Arc;

use serde_json::{Value, json};
use sts2_harness::management::{
    CommandKind, LiveWorkflowOptions, LiveWorkflowSessionFactory, MemoryWorkflowStore,
    WorkflowRunStatus,
};

#[path = "support/live_workflow.rs"]
mod support;

use support::*;

fn live_service_with(factory: &Arc<FakeFactory>) -> sts2_harness::management::ManagementService {
    live_service(
        Arc::new(MemoryWorkflowStore::new()),
        Arc::clone(factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )
    .expect("service")
}

/// The authored graph with an extra wired `observe` node between `observe` and `decide`, so
/// execution must visit every authored node rather than a hardcoded sequence.
fn alternate_definition() -> Value {
    let mut value = definition(false);
    let graph = &mut value["graphs"][0];
    let observe = graph["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .find(|node| node["id"] == "observe")
        .expect("observe node")
        .clone();
    let mut observe_again = observe;
    observe_again["id"] = json!("observe2");
    graph["nodes"]
        .as_array_mut()
        .expect("nodes")
        .push(observe_again);
    let edges = graph["edges"].as_array_mut().expect("edges");
    for edge in edges.iter_mut() {
        if edge["from"] == "observe" && edge["to"] == "decide" && edge["on"] == "ok" {
            edge["to"] = json!("observe2");
        }
    }
    edges.push(json!({"from": "observe2", "to": "decide", "on": "ok", "priority": 0}));
    value
}

#[test]
fn launch_binds_the_authored_graph_digest_and_node_order() {
    let factory = Arc::new(FakeFactory::new(false));
    let service = live_service_with(&factory);
    let actor = actor();
    let submitted = service
        .submit_run(&actor, request("request-bind-1", definition(false)))
        .expect("submit");
    let run_id = submitted.workflow_run_id;

    service
        .command(&actor, command(&run_id, "step-1", 1, CommandKind::Step))
        .expect("observe");
    service
        .command(&actor, command(&run_id, "step-2", 2, CommandKind::Step))
        .expect("decide");
    service
        .command(&actor, command(&run_id, "step-3", 3, CommandKind::Step))
        .expect("execute");
    service
        .command(&actor, command(&run_id, "step-4", 4, CommandKind::Step))
        .expect("terminal");
    assert_eq!(
        factory.entries(),
        [
            "launch",
            "observe",
            "legal_actions",
            "decide",
            "dispatch",
            "wait",
            "release"
        ]
    );

    let launches = factory.launches();
    assert_eq!(launches.len(), 1, "one live session was opened");
    let launch = &launches[0];
    assert_eq!(
        launch.definition_digest.len(),
        64,
        "the definition digest is a sha256 hex"
    );
    assert!(
        launch
            .definition_digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()),
        "the definition digest is lower-case hex"
    );
    assert_eq!(
        launch.node_order,
        [
            "observe".to_owned(),
            "decide".to_owned(),
            "execute".to_owned(),
            "done".to_owned(),
            "blocked".to_owned()
        ],
        "the launch binds the authored graph's node order"
    );

    // The same definition binds the same digest on a fresh live session.
    let repeat_factory = Arc::new(FakeFactory::new(false));
    let repeat_service = live_service_with(&repeat_factory);
    repeat_service
        .submit_run(&actor, request("request-bind-2", definition(false)))
        .expect("repeat submit");
    assert_eq!(
        repeat_factory.launches()[0].definition_digest,
        launch.definition_digest,
        "the same definition binds the same digest"
    );
}

#[test]
fn a_changed_graph_changes_the_launched_digest_and_execution() {
    let baseline_factory = Arc::new(FakeFactory::new(false));
    let baseline_service = live_service_with(&baseline_factory);
    let actor = actor();
    let baseline_run = baseline_service
        .submit_run(&actor, request("request-graph-base", definition(false)))
        .expect("baseline submit")
        .workflow_run_id;
    for (id, revision) in [("step-1", 1), ("step-2", 2), ("step-3", 3), ("step-4", 4)] {
        baseline_service
            .command(
                &actor,
                command(&baseline_run, id, revision, CommandKind::Step),
            )
            .expect("baseline step");
    }

    let changed_factory = Arc::new(FakeFactory::new(false));
    let changed_service = live_service_with(&changed_factory);
    let changed_run = changed_service
        .submit_run(
            &actor,
            request("request-graph-changed", alternate_definition()),
        )
        .expect("changed submit")
        .workflow_run_id;
    for (id, revision) in [
        ("step-1", 1),
        ("step-2", 2),
        ("step-3", 3),
        ("step-4", 4),
        ("step-5", 5),
    ] {
        changed_service
            .command(
                &actor,
                command(&changed_run, id, revision, CommandKind::Step),
            )
            .expect("changed step");
    }

    let baseline_launch = baseline_factory.launches();
    let changed_launch = changed_factory.launches();
    assert_eq!(baseline_launch.len(), 1);
    assert_eq!(changed_launch.len(), 1);
    assert_ne!(
        baseline_launch[0].definition_digest, changed_launch[0].definition_digest,
        "changing the graph must change the bound definition digest"
    );
    let baseline_observes = baseline_factory
        .entries()
        .iter()
        .filter(|entry| *entry == "observe")
        .count();
    let changed_observes = changed_factory
        .entries()
        .iter()
        .filter(|entry| *entry == "observe")
        .count();
    assert_eq!(baseline_observes, 1);
    assert_eq!(
        changed_observes, 2,
        "the changed graph visits the extra authored observe node"
    );
    assert_eq!(
        changed_service
            .status(&actor, &changed_run)
            .expect("status")
            .run
            .status,
        WorkflowRunStatus::Completed
    );
}

/// The service-side preflight rejects the tampered admission before any live effect. The
/// execution-boundary duplicate of these checks is covered by the unit tests in
/// `management::execution_admission` (the service preflight masks them end-to-end).
#[test]
fn service_target_admission_fails_closed_before_any_live_effect() {
    type Tamper = fn(&mut sts2_harness::management::TargetAdmissionBinding);
    let cases: [(&str, Tamper, &str); 4] = [
        (
            "instance",
            |admission| admission.target.instance_id = "instance-2".to_owned(),
            "target_instance_mismatch",
        ),
        (
            "stale_revision",
            |admission| admission.target.workflow_revision = "9.9.9".to_owned(),
            "target_admission_stale",
        ),
        (
            "definition_digest",
            |admission| admission.workflow_definition_digest = "0".repeat(64),
            "target_admission_digest_mismatch",
        ),
        (
            "request_identity",
            |admission| admission.request_id = "other-request".to_owned(),
            "target_request_mismatch",
        ),
    ];
    for (label, tamper, expected_code) in cases {
        let factory = Arc::new(FakeFactory::new(false));
        let service = live_service_with(&factory);
        let mut request = request("request-admission", definition(false));
        tamper(request.admission.as_mut().expect("live admission binding"));
        let error = service
            .submit_run(&actor(), request)
            .expect_err("a mismatched target admission must fail closed");
        assert_eq!(error.code, expected_code, "case {label}");
        assert!(
            factory.entries().is_empty(),
            "no live effect may occur for case {label}"
        );
    }
}
