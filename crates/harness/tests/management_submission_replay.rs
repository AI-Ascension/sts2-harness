// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use sts2_harness::management::{
    AuthContext, CapabilityPort, ErrorClass, LiveWorkflowOptions, LiveWorkflowSessionFactory,
    ManagementError, MemoryWorkflowStore, SqliteWorkflowStore, SubmissionRecoveryState,
    TargetCatalogResponse,
};

#[path = "support/live_workflow.rs"]
mod support;

#[derive(Default)]
struct CapabilityReads(AtomicUsize);
impl CapabilityPort for CapabilityReads {
    fn capabilities(&self) -> Result<serde_json::Value, ManagementError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(support::capabilities())
    }
    fn target_catalog(
        &self,
        actor: &AuthContext,
    ) -> Result<TargetCatalogResponse, ManagementError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        support::FakeFactory::new(false).target_catalog(actor)
    }
}

#[test]
fn replay_rechecks_current_scopes_and_run_prefix_without_external_readmission() {
    for failed in [false, true] {
        let store = Arc::new(MemoryWorkflowStore::new());
        let factory = Arc::new(if failed {
            support::FakeFactory::launch_error()
        } else {
            support::FakeFactory::new(false)
        });
        let capabilities = Arc::new(CapabilityReads::default());
        let service = support::live_service(store, factory.clone(), LiveWorkflowOptions::default())
            .expect("service")
            .with_capability_port(capabilities.clone());
        let request = support::request("scope-replay", support::definition(false));
        let original = service.submit_run(&support::actor(), request.clone());
        assert_eq!(
            original.is_err(),
            failed,
            "unexpected admission: {original:?}"
        );
        let retained = service
            .submission_recovery(&support::actor(), &request.request_id)
            .expect("owner recovery");
        assert!(retained.workflow_run_id.is_some());
        let calls = capabilities.0.load(Ordering::SeqCst);
        let effects = factory.entries();
        let restricted = AuthContext::with_run_prefix(
            "operator",
            ["workflow:*".to_owned()],
            Some("different-run".to_owned()),
        )
        .expect("same subject restricted to a different run");
        let denied = service
            .submit_run(&restricted, request.clone())
            .expect_err("POST replay cannot disclose a forbidden run");
        assert_eq!(denied.class, ErrorClass::Forbidden);
        assert_eq!(denied.code, "run_scope_denied");
        assert_eq!(
            service
                .submission_recovery(&restricted, &request.request_id)
                .expect_err("GET recovery also enforces the run prefix")
                .code,
            "run_scope_denied"
        );
        assert_eq!(
            service
                .submission_recovery(&restricted, "absent-request")
                .expect("no forbidden run to expose")
                .state,
            SubmissionRecoveryState::Missing
        );
        let foreign = AuthContext::with_run_prefix(
            "foreign",
            ["workflow:*".to_owned()],
            Some("different-run".to_owned()),
        )
        .expect("foreign actor");
        assert_eq!(
            service
                .submission_recovery(&foreign, &request.request_id)
                .expect("foreign claim must stay hidden before checking its run")
                .state,
            SubmissionRecoveryState::Missing
        );
        let no_live = AuthContext::new("operator", ["workflow:control".to_owned()])
            .expect("same subject with live scope revoked");
        assert_eq!(
            service
                .submit_run(&no_live, request.clone())
                .expect_err("revoked live scope blocks POST replay")
                .code,
            "live_scope_required"
        );
        assert_eq!(
            service
                .submission_recovery(&no_live, &request.request_id)
                .expect("live scope is not needed for an authorized read"),
            retained
        );
        let read_only = AuthContext::new("operator", ["workflow:read".to_owned()])
            .expect("same subject without control");
        assert_eq!(
            service
                .submit_run(&read_only, request.clone())
                .expect_err("revoked control scope blocks POST replay")
                .code,
            "missing_scope"
        );
        let replay = service.submit_run(&support::actor(), request);
        if failed {
            assert_eq!(
                replay.expect_err("reserved failure requires recovery").code,
                "live_submission_recovery_required"
            );
        } else {
            assert_eq!(
                replay
                    .expect("authorized retained acknowledgement")
                    .workflow_run_id,
                retained.workflow_run_id.expect("retained run")
            );
        }
        assert_eq!(capabilities.0.load(Ordering::SeqCst), calls);
        assert_eq!(factory.entries(), effects, "replay must not relaunch");
    }
}

struct Database(std::path::PathBuf);
impl Drop for Database {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.0.display()));
        }
    }
}

#[test]
fn a_confirmed_pre_reservation_store_refusal_is_cached_without_a_launch() {
    let path = Database(std::env::temp_dir().join(format!(
        "sts2-submission-refusal-{}.sqlite",
        uuid::Uuid::new_v4()
    )));
    let store = Arc::new(SqliteWorkflowStore::open(&path.0).expect("store"));
    let connection = rusqlite::Connection::open(&path.0).expect("fault connection");
    connection
        .execute_batch(
            "CREATE TRIGGER refuse_reservation BEFORE INSERT ON management_runs
         BEGIN SELECT RAISE(ABORT, 'injected pre-reservation refusal'); END;",
        )
        .expect("refuse only run reservation");
    let factory = Arc::new(support::FakeFactory::new(false));
    let capabilities = Arc::new(CapabilityReads::default());
    let service = support::live_service(store, factory.clone(), LiveWorkflowOptions::default())
        .expect("service")
        .with_capability_port(capabilities.clone());
    let request = support::request("refused-reservation", support::definition(false));
    let first = service
        .submit_run(&support::actor(), request.clone())
        .expect_err("reservation cannot commit");
    assert_eq!(first.class, ErrorClass::Store);
    assert_eq!(first.code, "store_sqlite");
    let retained = service
        .submission_recovery(&support::actor(), &request.request_id)
        .expect("cached refusal");
    assert_eq!(retained.state, SubmissionRecoveryState::Failed);
    assert!(retained.workflow_run_id.is_none());
    assert_eq!(
        retained.error.expect("original refusal"),
        first.error_response().error
    );
    assert!(factory.entries().is_empty());
    let calls = capabilities.0.load(Ordering::SeqCst);
    connection
        .execute_batch("DROP TRIGGER refuse_reservation")
        .expect("clear test fault");
    let replay = service
        .submit_run(&support::actor(), request)
        .expect_err("clearing the fault must not erase the original identity");
    assert_eq!(replay.error_response(), first.error_response());
    assert_eq!(capabilities.0.load(Ordering::SeqCst), calls);
    assert!(factory.entries().is_empty());
}
