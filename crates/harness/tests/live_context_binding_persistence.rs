// SPDX-License-Identifier: MIT

//! Synthetic owner evidence across real SQLite close/reopen; no provider or game.
#![allow(clippy::expect_used)]

use std::sync::Arc;

use sts2_harness::management::{
    AuthContext, CommandKind, CommandRequest, CommandResponse, ContextOwnerBinding,
    LiveWorkflowOptions, ManagementService, PendingOperationState, RecoveryAdmission, RunSnapshot,
    SqliteWorkflowStore, live_store,
};

#[path = "support/context_binding_history.rs"]
mod history_support;
#[path = "support/live_workflow.rs"]
mod support;

use history_support::{Database, RecordingOwner};

fn service(
    store: Arc<SqliteWorkflowStore>,
    factory: Arc<support::FakeFactory>,
    owner: Arc<RecordingOwner>,
) -> ManagementService {
    live_store(store, factory, LiveWorkflowOptions::default())
        .expect("live service")
        .with_context_binding_history()
        .expect("SQLite history")
        .with_context_owner_port(owner)
}

#[test]
fn historical_owner_binding_survives_reopen_and_replay_without_rebinding() {
    let database = Database::new("enabled");
    let store = Arc::new(SqliteWorkflowStore::open(&database.0).expect("open"));
    let factory = Arc::new(support::FakeFactory::new(false));
    let owner = Arc::new(RecordingOwner::default());
    let first = service(store.clone(), factory.clone(), owner.clone());
    let actor = support::actor();
    let run_id = first
        .submit_run(
            &actor,
            support::request("binding-persistence", support::definition(false)),
        )
        .expect("submit")
        .workflow_run_id;
    assert!(
        first
            .recorded_context_binding(&actor, &run_id, "live.node.2")
            .expect("not bound")
            .is_none()
    );
    first
        .command(
            &actor,
            support::command(&run_id, "observe", 1, CommandKind::Step),
        )
        .expect("observe");
    let decide = support::command(&run_id, "decide", 2, CommandKind::Step);
    let response = first.command(&actor, decide.clone()).expect("decide");
    let accepted = owner.0.lock().expect("accepted").clone();
    assert_eq!(accepted.len(), 1);
    let recorded = first
        .recorded_context_binding(&actor, &run_id, "live.node.2")
        .expect("read historical binding")
        .expect("bound");
    assert_eq!(recorded.binding, accepted[0]);
    assert_eq!(recorded.command_id, decide.command_id);
    assert_eq!(recorded.run_revision, response.run_revision);
    let current = assert_advanced_cursor(&first, &run_id, &recorded.binding);
    assert_read_denials(&first, &run_id);
    let calls = factory.entries();
    assert_eq!(
        first.command(&actor, decide.clone()).expect("replay"),
        response
    );
    assert_eq!(owner.0.lock().expect("accepted").len(), 1);
    assert_eq!(factory.entries(), calls);
    drop(first);
    assert_eq!(Arc::strong_count(&store), 1);
    drop(store);
    assert_reopened(&database, &recorded.binding, decide, response, current);
}

fn assert_advanced_cursor(
    first: &ManagementService,
    run_id: &str,
    binding: &ContextOwnerBinding,
) -> RunSnapshot {
    let actor = support::actor();
    let current = first.status(&actor, run_id).expect("current").run;
    assert_ne!(current.cursor.node_execution_id, binding.node_execution_id);
    assert!(
        first
            .recorded_context_binding(&actor, run_id, &current.cursor.node_execution_id)
            .expect("current unbound")
            .is_none()
    );
    current
}

fn assert_reopened(
    database: &Database,
    binding: &ContextOwnerBinding,
    decide: CommandRequest,
    response: CommandResponse,
    current: RunSnapshot,
) {
    let actor = support::actor();
    let run_id = binding.workflow_run_id.as_str();
    let reopened = Arc::new(SqliteWorkflowStore::open(&database.0).expect("reopen"));
    let replacement_factory = Arc::new(support::FakeFactory::new(false));
    let replacement_owner = Arc::new(RecordingOwner::default());
    let restarted = service(
        reopened,
        replacement_factory.clone(),
        replacement_owner.clone(),
    );
    let recovered = restarted
        .recorded_context_binding(&actor, run_id, "live.node.2")
        .expect("recovered")
        .expect("retained");
    assert_eq!(&recovered.binding, binding);
    assert_eq!(recovered.command_id, decide.command_id);
    assert_eq!(recovered.run_revision, response.run_revision);
    assert_eq!(
        restarted
            .command(&actor, decide)
            .expect("replay after reopen"),
        response
    );
    assert_eq!(
        restarted.status(&actor, run_id).expect("status").run,
        current
    );
    assert_read_denials(&restarted, run_id);
    assert!(
        replacement_owner
            .0
            .lock()
            .expect("replacement owner")
            .is_empty()
    );
    assert!(replacement_factory.entries().is_empty());
    // Historical metadata does not attach a current inspection/control owner.
    assert_eq!(
        restarted
            .context_association(&actor, run_id)
            .expect_err("historical binding does not attach current inspection")
            .code,
        "context_inspection_port_unavailable"
    );
}

fn assert_read_denials(service: &ManagementService, run_id: &str) {
    let reader =
        AuthContext::new("operator", ["workflow:read".to_owned()]).expect("metadata reader");
    assert!(
        service
            .recorded_context_binding(&reader, run_id, "live.node.2")
            .expect("metadata read permitted")
            .is_some()
    );
    assert_eq!(
        service
            .command(
                &reader,
                support::command(run_id, "reader-control", 3, CommandKind::Step),
            )
            .expect_err("metadata history never grants control")
            .code,
        "missing_scope"
    );
    let foreign_run = AuthContext::with_run_prefix(
        "operator",
        ["workflow:read".to_owned()],
        Some("foreign.run.".to_owned()),
    )
    .expect("foreign run actor");
    let no_grant = AuthContext::new("operator", Vec::<String>::new()).expect("no grant");
    let foreign_subject =
        AuthContext::new("foreign.operator", ["workflow:*".to_owned()]).expect("foreign subject");
    for (actor, code) in [
        (foreign_run, "run_scope_denied"),
        (no_grant, "missing_scope"),
        (foreign_subject, "context_history_subject"),
    ] {
        assert_eq!(
            service
                .recorded_context_binding(&actor, run_id, "live.node.2")
                .expect_err("denied")
                .code,
            code,
            "historical owner facts require current read scope and original subject"
        );
    }
}

#[test]
fn ordinary_sqlite_composition_does_not_retain_owner_history() {
    let database = Database::new("disabled");
    let store = Arc::new(SqliteWorkflowStore::open(&database.0).expect("open"));
    let factory = Arc::new(support::FakeFactory::new(false));
    let owner = Arc::new(RecordingOwner::default());
    let ordinary = live_store(
        store.clone(),
        factory.clone(),
        LiveWorkflowOptions::default(),
    )
    .expect("ordinary service")
    .with_context_owner_port(owner.clone());
    let actor = support::actor();
    let run_id = ordinary
        .submit_run(
            &actor,
            support::request("binding-no-retention", support::definition(false)),
        )
        .expect("submit")
        .workflow_run_id;
    for (id, revision) in [("observe", 1), ("decide", 2)] {
        ordinary
            .command(
                &actor,
                support::command(&run_id, id, revision, CommandKind::Step),
            )
            .expect("step");
    }
    assert_eq!(owner.0.lock().expect("owner").len(), 1);
    assert_eq!(
        ordinary
            .recorded_context_binding(&actor, &run_id, "live.node.2")
            .expect_err("not enabled")
            .code,
        "context_history_unavailable"
    );
    drop(ordinary);
    assert_eq!(Arc::strong_count(&store), 1);
    drop(store);
    let reopened = Arc::new(SqliteWorkflowStore::open(&database.0).expect("reopen"));
    let enabled = service(reopened, factory, owner);
    assert!(
        enabled
            .recorded_context_binding(&actor, &run_id, "live.node.2")
            .expect("absent history")
            .is_none()
    );
}

#[test]
fn command_result_failure_rolls_back_history_and_does_not_repeat_effects() {
    let database = Database::new("rollback");
    let store = Arc::new(SqliteWorkflowStore::open(&database.0).expect("open"));
    let factory = Arc::new(support::FakeFactory::new(false));
    let owner = Arc::new(RecordingOwner::default());
    let service = service(store, factory.clone(), owner.clone());
    let actor = support::actor();
    let run_id = submit_observed(&service, "binding-rollback");
    let before = service.status(&actor, &run_id).expect("before").run;
    let connection = rusqlite::Connection::open(&database.0).expect("fault connection");
    connection
        .execute_batch(
            "CREATE TRIGGER fail_result BEFORE UPDATE ON management_commands
             WHEN NEW.response IS NOT NULL BEGIN SELECT RAISE(ABORT, 'fixture result fault'); END;",
        )
        .expect("install result fault");
    let decide = support::command(&run_id, "decide", 2, CommandKind::Step);
    assert!(service.command(&actor, decide.clone()).is_err());
    assert_eq!(owner.0.lock().expect("accepted").len(), 1);
    assert!(
        service
            .recorded_context_binding(&actor, &run_id, "live.node.2")
            .expect("history rolled back")
            .is_none()
    );
    // The rolled-back command application is not the only durable record of this step: the decision
    // attempt's intent crossed the provider boundary before it and is already committed, exactly as
    // a dispatched action's intent is. So the stored snapshot is the pre-command snapshot plus that
    // attempt, and the run advertises an outstanding effect instead of looking untouched.
    let after = service.status(&actor, &run_id).expect("after");
    let intent = after
        .run
        .pending_operation
        .clone()
        .expect("the decision attempt stays durable after the rollback");
    assert_eq!(intent.classification, PendingOperationState::Intent);
    assert_eq!(intent.instance_id, "instance-1");
    assert_eq!(intent.original_generation, 0);
    assert_eq!(after.recovery_admission, RecoveryAdmission::Reconciling);
    let mut expected = before;
    expected.pending_operation = Some(intent);
    assert_eq!(after.run, expected);
    let calls = factory.entries();
    connection
        .execute_batch("DROP TRIGGER fail_result")
        .expect("remove fault");
    assert_eq!(
        service.command(&actor, decide).expect("retry").outcome,
        sts2_harness::management::CommandOutcome::Pending
    );
    assert_eq!(owner.0.lock().expect("accepted").len(), 1);
    assert_eq!(factory.entries(), calls);
}

fn submit_observed(service: &ManagementService, request_id: &str) -> String {
    let actor = support::actor();
    let run_id = service
        .submit_run(
            &actor,
            support::request(request_id, support::definition(false)),
        )
        .expect("submit")
        .workflow_run_id;
    service
        .command(
            &actor,
            support::command(&run_id, "observe", 1, CommandKind::Step),
        )
        .expect("observe");
    run_id
}

#[test]
fn full_history_blocks_steps_before_effects_and_still_allows_cancel() {
    let database = Database::new("capacity");
    let store = Arc::new(SqliteWorkflowStore::open(&database.0).expect("open"));
    let factory = Arc::new(support::FakeFactory::new(false));
    let owner = Arc::new(RecordingOwner::default());
    let service = service(store, factory.clone(), owner.clone());
    let actor = support::actor();
    let run_id = submit_observed(&service, "binding-capacity");
    service
        .command(
            &actor,
            support::command(&run_id, "decide", 2, CommandKind::Step),
        )
        .expect("decide");
    fill_history(&database, &run_id);
    let calls = factory.entries();
    assert_eq!(
        service
            .command(
                &actor,
                support::command(&run_id, "over-capacity", 3, CommandKind::Step),
            )
            .expect_err("bounded before execution")
            .code,
        "context_history_limit"
    );
    assert_eq!(factory.entries(), calls);
    assert_eq!(owner.0.lock().expect("accepted").len(), 1);
    service
        .command(
            &actor,
            support::command(&run_id, "cancel", 3, CommandKind::Cancel),
        )
        .expect("cancel remains possible");
    assert_eq!(
        factory.entries().last().map(String::as_str),
        Some("release")
    );
}

fn fill_history(database: &Database, run_id: &str) {
    let mut connection = rusqlite::Connection::open(&database.0).expect("capacity connection");
    let transaction = connection.transaction().expect("fixture transaction");
    let source: Vec<u8> = transaction
        .query_row(
            "SELECT record FROM management_context_binding_history WHERE workflow_run_id = ?1",
            [run_id],
            |row| row.get(0),
        )
        .expect("original record");
    for index in 0..255 {
        let command_id = format!("fixture.command.{index}");
        let node_id = format!("fixture.node.{index}");
        let mut record: serde_json::Value = serde_json::from_slice(&source).expect("record");
        record["command_id"] = serde_json::json!(command_id);
        record["binding"]["node_execution_id"] = serde_json::json!(node_id);
        transaction
            .execute(
                "INSERT INTO management_commands
             SELECT workflow_run_id, ?2, request_digest, application_in_flight,
                    CAST(json_set(CAST(response AS TEXT), '$.command_id', ?2) AS BLOB)
             FROM management_commands WHERE workflow_run_id = ?1 AND command_id = 'decide'",
                rusqlite::params![run_id, command_id],
            )
            .expect("synthetic command capacity row");
        transaction
            .execute(
                "INSERT INTO management_context_binding_history VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    run_id,
                    node_id,
                    command_id,
                    serde_json::to_vec(&record).expect("encode")
                ],
            )
            .expect("synthetic history capacity row");
    }
    transaction.commit().expect("fixture capacity commit");
}
