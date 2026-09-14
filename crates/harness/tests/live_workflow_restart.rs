// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::sync::Arc;

use sts2_harness::management::{
    CommandKind, LiveWorkflowOptions, LiveWorkflowSessionFactory, MemoryWorkflowStore,
    RecoveryAdmission, SqliteWorkflowStore, WorkflowRunStatus, WorkflowStore, live_store,
};

#[path = "support/live_workflow.rs"]
mod support;

use support::*;

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
