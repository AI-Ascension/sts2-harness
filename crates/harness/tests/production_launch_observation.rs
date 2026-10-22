// SPDX-License-Identifier: MIT

//! Real production owner composition with synthetic runtime/provider ports, never native evidence.

#![allow(clippy::expect_used)]

use std::path::PathBuf;
use std::sync::{Arc, atomic::Ordering};
use sts2_harness::management::{
    CleanupState, ErrorClass, ErrorResponse, LiveWorkflowOptions, ManagementClient,
    ManagementServer, ManagementService, RecoveryAdmission, ServerConfig, SqliteWorkflowStore,
    StaticAuthenticator, SubmissionRecoveryResponse, SubmissionRecoveryState, WorkflowRunStatus,
    WorkflowStore,
};

#[path = "support/production_launch_fence.rs"]
mod fence;
#[path = "support/live_workflow.rs"]
mod support;

struct Database(PathBuf);
impl Drop for Database {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.0.display()));
        }
    }
}

fn assert_one_attempt(counts: &fence::Counts) {
    assert_eq!(counts.opens.load(Ordering::SeqCst), 1);
    assert_eq!(counts.launches.load(Ordering::SeqCst), 1);
    assert_eq!(counts.observations.load(Ordering::SeqCst), 1);
    assert_eq!(counts.providers.load(Ordering::SeqCst), 0);
    assert_eq!(counts.stops.load(Ordering::SeqCst), 1);
    assert_eq!(counts.releases.load(Ordering::SeqCst), 1);
}

fn exercise_failure(
    retryable: bool,
    cleanup_failure: bool,
    persistence_failure: bool,
    expected_class: ErrorClass,
    expected_code: &str,
) {
    let database = Database(std::env::temp_dir().join(format!(
        "sts2-production-observe-{}.sqlite",
        uuid::Uuid::new_v4()
    )));
    let store = Arc::new(SqliteWorkflowStore::open(&database.0).expect("SQLite store"));
    let fault = rusqlite::Connection::open(&database.0).expect("fault connection");
    if persistence_failure {
        fault
            .execute_batch(
                "CREATE TRIGGER refuse_status_update BEFORE UPDATE ON management_runs
             BEGIN SELECT RAISE(ABORT, 'injected status persistence failure'); END;",
            )
            .expect("refuse status updates but preserve the committed reservation");
    }
    let counts = Arc::new(fence::Counts::default());
    let factory = fence::factory(store.clone(), counts.clone(), retryable, cleanup_failure);
    let service = Arc::new(
        support::live_service(
            store.clone(),
            factory.clone(),
            LiveWorkflowOptions::default(),
        )
        .expect("served production service"),
    );
    let auth = StaticAuthenticator::single("fence-token", support::actor()).expect("auth");
    let config =
        ServerConfig::new("127.0.0.1:0".parse().expect("address"), Arc::new(auth)).expect("config");
    let server = ManagementServer::start(config, service.clone()).expect("server");
    let client = ManagementClient::new(server.address(), "fence-token").expect("client");
    let request = support::request("production-observe-failure", support::definition(false));
    let bytes = serde_json::to_vec(&request).expect("request");
    let response = client
        .request_json("POST", "/v1/workflow-runs", Some(&bytes))
        .expect("production refusal reaches HTTP peer");
    let expected_status = if expected_class == ErrorClass::Unresolved {
        409
    } else {
        503
    };
    assert_eq!(response.status, expected_status);
    let error: ErrorResponse = serde_json::from_slice(&response.body).expect("error envelope");
    assert_eq!(
        error.schema_version,
        if expected_class == ErrorClass::WriteStateUnknown {
            "ascension.management-error/v2"
        } else {
            "ascension.management/v1"
        }
    );
    assert_eq!(
        error.error.class,
        if expected_class == ErrorClass::Unresolved {
            ErrorClass::Conflict
        } else {
            expected_class.clone()
        }
    );
    assert_eq!(error.error.code, expected_code);
    if !persistence_failure {
        assert!(
            error
                .error
                .message
                .contains("recover the original request_id")
        );
        assert!(error.error.message.contains("do not resubmit"));
    }
    if cleanup_failure {
        assert!(
            error
                .error
                .message
                .contains("injected launch observation failure")
        );
        assert!(error.error.message.contains("cleanup failed"));
    }
    if persistence_failure {
        assert!(error.error.message.contains("live_launch_fence_failed"));
    }
    assert_one_attempt(&counts);

    let response = client
        .request_json(
            "GET",
            &format!("/v1/workflow-submissions/{}", request.request_id),
            None,
        )
        .expect("original request recovery");
    assert_eq!(response.status, 200);
    let recovery: SubmissionRecoveryResponse =
        serde_json::from_slice(&response.body).expect("typed recovery");
    assert_eq!(recovery.state, SubmissionRecoveryState::Failed);
    // HTTP v1 keeps the legacy conflict projection, while durable recovery preserves
    // the owner's precise classification and the otherwise identical diagnostic.
    let mut expected_owner_error = error.error.clone();
    expected_owner_error.class = expected_class;
    assert_eq!(recovery.error.as_ref(), Some(&expected_owner_error));
    let run_id = recovery
        .workflow_run_id
        .as_ref()
        .expect("reserved run remains visible");
    let snapshot = store
        .get_run(run_id)
        .expect("stored reservation")
        .expect("run row");
    assert_eq!(
        snapshot.status,
        if persistence_failure {
            WorkflowRunStatus::Created
        } else {
            WorkflowRunStatus::NeedsOperator
        }
    );
    assert_eq!(
        snapshot.cleanup,
        if persistence_failure {
            CleanupState::NotStarted
        } else {
            CleanupState::NeedsOperator
        }
    );
    assert_eq!(
        service
            .status(&support::actor(), run_id)
            .expect("run recovery status")
            .recovery_admission,
        RecoveryAdmission::NeedsOperator
    );

    let repeated = client
        .request_json("POST", "/v1/workflow-runs", Some(&bytes))
        .expect("repeated POST refusal");
    assert_eq!(repeated.status, 503);
    let repeated: ErrorResponse = serde_json::from_slice(&repeated.body).expect("repeat error");
    assert_eq!(repeated.error.class, ErrorClass::WriteStateUnknown);
    assert_eq!(repeated.error.code, "live_submission_recovery_required");
    assert_one_attempt(&counts);
    server.shutdown().expect("server shutdown");
    drop(service);
    drop(factory);
    drop(store);
    drop(fault);

    // A fresh service has no live execution sessions. The durable claim, original error,
    // and reservation remain authoritative after every old store connection is dropped.
    let reopened = Arc::new(SqliteWorkflowStore::open(&database.0).expect("reopened store"));
    let restarted = ManagementService::new(reopened.clone());
    assert_eq!(
        restarted
            .submission_recovery(&support::actor(), &request.request_id)
            .expect("restart recovery"),
        recovery
    );
    assert!(
        reopened
            .get_run(run_id)
            .expect("restart reservation")
            .is_some()
    );
    assert_eq!(
        restarted
            .submit_run(&support::actor(), request)
            .expect_err("restart cannot grant another launch")
            .code,
        "live_submission_recovery_required"
    );
    assert_one_attempt(&counts);
}

#[test]
fn production_http_distinguishes_retryable_and_terminal_observe_failures_without_relaunch() {
    exercise_failure(
        true,
        false,
        false,
        ErrorClass::WriteStateUnknown,
        "live_launch_fence_failed",
    );
    exercise_failure(
        false,
        false,
        false,
        ErrorClass::Unresolved,
        "live_launch_fence_failed",
    );
}

#[test]
fn cleanup_and_persistence_faults_keep_their_diagnostics_and_original_reservation() {
    exercise_failure(
        true,
        true,
        false,
        ErrorClass::Unavailable,
        "live_launch_cleanup_failed",
    );
    exercise_failure(
        false,
        false,
        true,
        ErrorClass::Store,
        "reservation_failure_persist",
    );
}
