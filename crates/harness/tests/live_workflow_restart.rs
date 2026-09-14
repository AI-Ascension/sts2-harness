// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::sync::Arc;

use sts2_harness::management::{
    AuthContext, CommandApplication, CommandContext, CommandKind, CommandOutcome,
    LiveWorkflowExecutionPort, LiveWorkflowOptions, LiveWorkflowSessionFactory, ManagementError,
    MemoryWorkflowStore, PendingOperation, PendingOperationState, RecoveryAdmission, RunAdmission,
    RunRequest, RunReservation, SqliteWorkflowStore, TargetAdmissionBinding, WorkflowExecutionPort,
    WorkflowRunStatus, WorkflowStore, live_store,
};

#[path = "support/live_workflow.rs"]
mod support;

use support::*;

#[test]
fn durable_unknown_intent_precedes_live_resume_after_apply_failure_and_restart() {
    let store = Arc::new(MemoryWorkflowStore::new());
    let factory = Arc::new(support::FakeFactory::new(true));
    let service = apply_failure_service(Arc::clone(&store), Arc::clone(&factory));
    let request = request("request-live-pending-apply-failure", definition(false));
    let submitted = service
        .submit_run(&actor(), request.clone())
        .expect("submit");
    let run_id = submitted.workflow_run_id.clone();
    for (id, revision) in [("step-1", 1), ("step-2", 2)] {
        service
            .command(&actor(), command(&run_id, id, revision, CommandKind::Step))
            .expect("step");
    }
    let pending = service
        .command(&actor(), command(&run_id, "step-3", 3, CommandKind::Step))
        .expect_err("store application failure after unknown effect");
    assert_eq!(pending.code, "port_identity_mismatch");
    let snapshot = store.get_run(&run_id).expect("lookup").expect("run");
    assert_eq!(
        snapshot
            .pending_operation
            .as_ref()
            .expect("durable operation intent")
            .classification,
        PendingOperationState::Intent
    );
    assert_eq!(snapshot.status, WorkflowRunStatus::Running);
    assert_eq!(
        service
            .status(&actor(), &run_id)
            .expect("same-process status")
            .recovery_admission,
        RecoveryAdmission::Reconciling
    );
    drop(service);

    let restarted_factory = Arc::new(support::FakeFactory::new(false));
    let restarted = apply_failure_service(Arc::clone(&store), restarted_factory.clone());
    assert_eq!(
        restarted
            .status(&actor(), &run_id)
            .expect("restarted status")
            .recovery_admission,
        RecoveryAdmission::Reconciling
    );
    let response = restarted
        .command(&actor(), command(&run_id, "step-4", 4, CommandKind::Step))
        .expect("restarted unresolved effect remains pending");
    assert_eq!(response.outcome, CommandOutcome::Pending);
    assert_eq!(response.run_revision, 3);
    assert_eq!(response.sequence, None);
    assert!(restarted_factory.entries().is_empty());
}

fn apply_failure_service(
    store: Arc<MemoryWorkflowStore>,
    factory: Arc<support::FakeFactory>,
) -> sts2_harness::management::ManagementService {
    let store_port: Arc<dyn WorkflowStore> = Arc::clone(&store) as Arc<dyn WorkflowStore>;
    let factory_port: Arc<dyn LiveWorkflowSessionFactory> =
        Arc::clone(&factory) as Arc<dyn LiveWorkflowSessionFactory>;
    let service = live_store(
        Arc::clone(&store_port),
        Arc::clone(&factory_port),
        LiveWorkflowOptions::default(),
    )
    .expect("service");
    let execution = LiveWorkflowExecutionPort::new(factory_port, LiveWorkflowOptions::default())
        .expect("execution");
    service.with_execution_port(Arc::new(ApplyFailureExecution {
        inner: Arc::new(execution),
    }))
}

struct ApplyFailureExecution {
    inner: Arc<LiveWorkflowExecutionPort>,
}

impl WorkflowExecutionPort for ApplyFailureExecution {
    fn submit(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition_digest: &str,
    ) -> Result<RunAdmission, ManagementError> {
        self.inner.submit(request, actor, definition_digest)
    }

    fn submit_admitted(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition_digest: &str,
        admission: Option<&TargetAdmissionBinding>,
    ) -> Result<RunAdmission, ManagementError> {
        self.inner
            .submit_admitted(request, actor, definition_digest, admission)
    }

    fn submit_admitted_with_reservation(
        &self,
        request: &RunRequest,
        actor: &AuthContext,
        definition_digest: &str,
        admission: Option<&TargetAdmissionBinding>,
        reservation: &RunReservation,
    ) -> Result<RunAdmission, ManagementError> {
        self.inner.submit_admitted_with_reservation(
            request,
            actor,
            definition_digest,
            admission,
            reservation,
        )
    }

    fn recovery_admission(
        &self,
        snapshot: &sts2_harness::management::RunSnapshot,
    ) -> Option<RecoveryAdmission> {
        self.inner.recovery_admission(snapshot)
    }

    fn abort_submission(&self, run_id: &str) -> Result<(), ManagementError> {
        self.inner.abort_submission(run_id)
    }

    fn apply_command(
        &self,
        context: CommandContext,
    ) -> Result<CommandApplication, ManagementError> {
        self.inner.apply_command(context)
    }

    fn apply_command_with_intent(
        &self,
        context: CommandContext,
        record_intent: &dyn Fn(PendingOperation) -> Result<(), ManagementError>,
    ) -> Result<CommandApplication, ManagementError> {
        let mut application = self
            .inner
            .apply_command_with_intent(context.clone(), record_intent)?;
        if application.outcome == CommandOutcome::Pending {
            application.snapshot.workflow_run_id = format!("{}-mismatch", context.request.run_id);
        }
        Ok(application)
    }
}

#[test]
fn successful_live_snapshot_after_memory_restart_requires_operator() {
    let store = Arc::new(MemoryWorkflowStore::new());
    let factory = Arc::new(support::FakeFactory::new(false));
    let service = live_store(
        Arc::clone(&store) as Arc<dyn WorkflowStore>,
        Arc::clone(&factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )
    .expect("service");
    let request = request("request-live-restart-running-memory", definition(false));
    let submitted = service.submit_run(&actor(), request).expect("submit");
    assert_eq!(submitted.status, WorkflowRunStatus::Running);
    assert_eq!(
        service
            .status(&actor(), &submitted.workflow_run_id)
            .expect("status")
            .recovery_admission,
        RecoveryAdmission::SafelyResumable {
            capability: "live.workflow.resume.v1".to_owned(),
        }
    );
    let run_id = submitted.workflow_run_id;
    drop(service);

    let restarted_factory = Arc::new(support::FakeFactory::new(false));
    let restarted = live_store(
        Arc::clone(&store) as Arc<dyn WorkflowStore>,
        Arc::clone(&restarted_factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )
    .expect("restarted service");
    let status = restarted.status(&actor(), &run_id).expect("status");
    assert_eq!(status.run.status, WorkflowRunStatus::Running);
    assert_eq!(status.recovery_admission, RecoveryAdmission::NeedsOperator);
    let error = restarted
        .command(
            &actor(),
            command(&run_id, "restart-step-memory", 1, CommandKind::Step),
        )
        .expect_err("restarted live runtime must fail closed");
    assert_eq!(error.code, "live_runtime_after_restart");
    assert!(restarted_factory.entries().is_empty());
}

#[test]
fn successful_live_snapshot_after_sqlite_restart_requires_operator()
-> Result<(), Box<dyn std::error::Error>> {
    let directory =
        std::env::temp_dir().join(format!("sts2-live-restart-running-{}", std::process::id()));
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("live.sqlite3");
    let store = Arc::new(SqliteWorkflowStore::open(&path)?);
    let factory = Arc::new(support::FakeFactory::new(false));
    let service = live_store(
        Arc::clone(&store) as Arc<dyn WorkflowStore>,
        Arc::clone(&factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )?;
    let request = request("request-live-restart-running-sqlite", definition(false));
    let submitted = service.submit_run(&actor(), request)?;
    assert_eq!(submitted.status, WorkflowRunStatus::Running);
    assert_eq!(
        service
            .status(&actor(), &submitted.workflow_run_id)?
            .recovery_admission,
        RecoveryAdmission::SafelyResumable {
            capability: "live.workflow.resume.v1".to_owned(),
        }
    );
    let run_id = submitted.workflow_run_id;
    drop(service);
    drop(store);

    let restarted_store = Arc::new(SqliteWorkflowStore::open(&path)?);
    let restarted_factory = Arc::new(support::FakeFactory::new(false));
    let restarted = live_store(
        Arc::clone(&restarted_store) as Arc<dyn WorkflowStore>,
        Arc::clone(&restarted_factory) as Arc<dyn LiveWorkflowSessionFactory>,
        LiveWorkflowOptions::default(),
    )?;
    let status = restarted.status(&actor(), &run_id)?;
    assert_eq!(status.run.status, WorkflowRunStatus::Running);
    assert_eq!(status.recovery_admission, RecoveryAdmission::NeedsOperator);
    let error = restarted
        .command(
            &actor(),
            command(&run_id, "restart-step-sqlite", 1, CommandKind::Step),
        )
        .expect_err("restarted live runtime must fail closed");
    assert_eq!(error.code, "live_runtime_after_restart");
    assert!(restarted_factory.entries().is_empty());
    drop(restarted);
    drop(restarted_store);
    std::fs::remove_dir_all(directory)?;
    Ok(())
}
