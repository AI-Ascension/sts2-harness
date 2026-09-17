// SPDX-License-Identifier: MIT

//! Durability half of the sts2-harness#99 admission matrix: the live adapter's
//! catalog re-check immediately before a session is opened (after the durable
//! reservation), and idempotent identity recovery across a management restart
//! over SQLite.
//!
//! Deterministic component evidence over synthetic doubles only. No gateway,
//! provider, native game or host is involved; nothing here proves native
//! behaviour.

#![allow(clippy::expect_used)]

use std::sync::Arc;

use sts2_harness::management::{
    CleanupState, LiveWorkflowOptions, LiveWorkflowSessionFactory, MemoryWorkflowStore,
    TargetAvailability, TargetCatalogResponse, WorkflowRunStatus, WorkflowStore, digest_value,
    live_run_id,
};

#[path = "support/target_catalog_matrix_service.rs"]
mod fixture;
#[path = "support/target_catalog_matrix.rs"]
mod matrix;
#[path = "support/live_workflow.rs"]
mod support;

use fixture::{Outcome, operator, preflight, restart_fixture, run_request, status_snapshot};
use matrix::{Database, DriftingFactory, beta_descriptor, persisted_admission, selection};
use support::{actor, definition, live_service};

type CatalogDrift = fn(&mut TargetCatalogResponse);

#[test]
fn live_port_recheck_immediately_before_open_rejects_drift_without_effects() -> Outcome {
    let good = support::FakeFactory::new(false).target_catalog(&actor())?;
    let cases: [(&str, CatalogDrift, &str); 6] = [
        (
            "removed",
            |catalog| catalog.targets.clear(),
            "target_unavailable",
        ),
        (
            "revoked",
            |catalog| catalog.targets[0].availability = TargetAvailability::Revoked,
            "target_revoked",
        ),
        (
            "expired",
            |catalog| catalog.targets[0].availability = TargetAvailability::Expired,
            "target_expired",
        ),
        (
            "compatibility",
            |catalog| {
                catalog.targets[0].compatibility_revision = "live.compatibility.v2".to_owned()
            },
            "target_compatibility_stale",
        ),
        (
            "catalog-revision",
            |catalog| catalog.catalog_revision = "live.catalog.v2".to_owned(),
            "target_catalog_stale",
        ),
        (
            "descriptor",
            |catalog| {
                catalog.targets[0]
                    .save_profiles
                    .push("save-late".to_owned())
            },
            "target_descriptor_stale",
        ),
    ];
    for (label, drift, expected) in cases {
        let mut drifted = good.clone();
        drift(&mut drifted);
        // Call 1: management revalidation; call 2: adapter check before the
        // durable reservation; call 3: adapter re-check immediately before open.
        for (drift_at, reserved) in [(1usize, false), (2usize, true)] {
            let mut script = vec![good.clone(); drift_at];
            script.push(drifted.clone());
            let store = Arc::new(MemoryWorkflowStore::new());
            let factory = DriftingFactory::new(script);
            let service = live_service(
                Arc::clone(&store) as Arc<dyn WorkflowStore>,
                Arc::clone(&factory) as Arc<dyn LiveWorkflowSessionFactory>,
                LiveWorkflowOptions::default(),
            )?;
            let request = support::request(
                &format!("request-live-{label}-{drift_at}"),
                definition(false),
            );
            let run_id = live_run_id(&request, &digest_value(&definition(false))?)?;
            let error = service
                .submit_run(&actor(), request.clone())
                .expect_err("drift before open must reject");
            assert_eq!(error.code, expected, "{label} at call {}", drift_at + 1);
            assert_eq!(factory.calls(), drift_at + 1, "{label}");
            assert!(
                factory.inner.launches().is_empty(),
                "{label}: no session opened"
            );
            assert!(
                factory.inner.entries().is_empty(),
                "{label}: no live effect"
            );
            let persisted = store.get_run(&run_id)?;
            if reserved {
                let snapshot = persisted.expect("reservation persisted before the final check");
                assert_eq!(snapshot.status, WorkflowRunStatus::NeedsOperator, "{label}");
                assert_eq!(snapshot.cleanup, CleanupState::NeedsOperator, "{label}");
                assert_eq!(snapshot.admission, request.admission, "{label}");
                let retry = service
                    .submit_run(&actor(), request)
                    .expect_err("a failed reservation is not an idempotent success");
                assert_eq!(retry.code, "live_submission_recovery_required", "{label}");
            } else {
                assert!(persisted.is_none(), "{label}: nothing reserved");
            }
        }
    }
    Ok(())
}

#[test]
fn duplicate_submission_after_management_restart_returns_one_run_with_unchanged_admission()
-> Outcome {
    let database = Database::new();
    let restarted = restart_fixture(&database, "request-restart")?;
    let full = operator("operator");
    let expected = restarted.request.admission.clone();
    let before = status_snapshot(&restarted, &full)?;
    assert_eq!(before.admission, expected);

    let after = restarted
        .service
        .submit_run(&full, restarted.request.clone())?;
    assert_eq!(after, restarted.first);
    assert_eq!(
        restarted.port.submissions(),
        0,
        "restart recovery must not resubmit"
    );
    let snapshot = status_snapshot(&restarted, &full)?;
    assert_eq!(snapshot, before);
    assert_eq!(serde_json::to_vec(&snapshot)?, serde_json::to_vec(&before)?);
    assert_eq!(
        persisted_admission(restarted.store.as_ref(), &after.workflow_run_id),
        expected
    );
    // Identity recovery is a receipt lookup; the catalog is not consulted again.
    let calls = restarted.catalog.calls();
    assert_eq!(
        restarted
            .service
            .submit_run(&full, restarted.request.clone())?,
        after
    );
    assert_eq!(restarted.catalog.calls(), calls);
    Ok(())
}

#[test]
fn mismatched_idempotency_payload_after_restart_conflicts() -> Outcome {
    let database = Database::new();
    let restarted = restart_fixture(&database, "request-mismatch")?;
    let full = operator("operator");
    let before = status_snapshot(&restarted, &full)?;

    let beta = preflight(
        &restarted.service,
        &full,
        "request-mismatch",
        selection(&beta_descriptor(), "live.workflow.v2"),
    )?;
    let other_target = run_request("request-mismatch", beta);
    let mut other_definition = restarted.request.clone();
    other_definition.definition.as_mut().expect("definition")["limits"]["max_steps"] =
        serde_json::json!(31);
    let mut other_binding = restarted.request.clone();
    other_binding
        .admission
        .as_mut()
        .expect("admission")
        .catalog_revision = "matrix.catalog.v2".to_owned();
    for (label, changed) in [
        ("target", other_target),
        ("definition", other_definition),
        ("binding", other_binding),
    ] {
        let error = restarted
            .service
            .submit_run(&full, changed)
            .expect_err("same request_id with a different payload must conflict");
        assert_eq!(error.code, "submission_conflict", "{label}");
    }
    assert_eq!(restarted.port.submissions(), 0);
    assert_eq!(status_snapshot(&restarted, &full)?, before);
    Ok(())
}
