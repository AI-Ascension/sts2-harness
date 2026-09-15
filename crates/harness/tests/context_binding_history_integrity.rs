// SPDX-License-Identifier: MIT

//! Corrupt-store negatives with synthetic bindings; no native or provider calls.
#![allow(clippy::expect_used)]

use std::sync::Arc;

use rusqlite::{Connection, params};
use serde_json::{Value, json};
use sts2_harness::management::{
    CommandKind, CommandRequest, LiveWorkflowOptions, ManagementService, SqliteWorkflowStore,
    live_store,
};

#[path = "support/context_binding_history.rs"]
mod history_support;
#[path = "support/live_workflow.rs"]
mod support;

use history_support::{Database, RecordingOwner};

struct Fixture {
    // Keep the database cleanup guard last, after all database connections.
    connection: Connection,
    service: ManagementService,
    owner: Arc<RecordingOwner>,
    factory: Arc<support::FakeFactory>,
    decide: CommandRequest,
    record: Vec<u8>,
    response: Vec<u8>,
    _database: Database,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let database = Database::new(name);
        let store = Arc::new(SqliteWorkflowStore::open(&database.0).expect("open"));
        let owner = Arc::new(RecordingOwner::default());
        let factory = Arc::new(support::FakeFactory::new(false));
        let service = live_store(store, factory.clone(), LiveWorkflowOptions::default())
            .expect("service")
            .with_context_binding_history()
            .expect("history")
            .with_context_owner_port(owner.clone());
        let actor = support::actor();
        let run_id = service
            .submit_run(&actor, support::request(name, support::definition(false)))
            .expect("submit")
            .workflow_run_id;
        service
            .command(
                &actor,
                support::command(&run_id, "observe", 1, CommandKind::Step),
            )
            .expect("observe");
        let decide = support::command(&run_id, "decide", 2, CommandKind::Step);
        service.command(&actor, decide.clone()).expect("decide");
        let connection = Connection::open(&database.0).expect("corruption connection");
        let (record, response) = connection
            .query_row(
                "SELECT history.record, command.response
                 FROM management_context_binding_history AS history
                 JOIN management_commands AS command USING (workflow_run_id, command_id)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("original bytes");
        Self {
            connection,
            service,
            owner,
            factory,
            decide,
            record,
            response,
            _database: database,
        }
    }

    fn replace_record(&self, bytes: &[u8]) {
        assert_eq!(
            self.connection
                .execute(
                    "UPDATE management_context_binding_history SET record = ?1",
                    [bytes],
                )
                .expect("replace record"),
            1
        );
    }

    fn replace_response(&self, bytes: &[u8]) {
        assert_eq!(
            self.connection
                .execute(
                    "UPDATE management_commands SET response = ?1 WHERE command_id = ?2",
                    params![bytes, self.decide.command_id],
                )
                .expect("replace response"),
            1
        );
    }

    fn assert_rejected_without_effects(&self, case: &str) {
        let calls = self.factory.entries();
        let bindings = self.owner.0.lock().expect("bindings").clone();
        assert!(
            self.service
                .recorded_context_binding(&support::actor(), &self.decide.run_id, "live.node.2",)
                .is_err(),
            "corrupt persisted {case} must fail closed"
        );
        assert_eq!(self.factory.entries(), calls);
        assert_eq!(*self.owner.0.lock().expect("bindings"), bindings);
    }

    fn assert_restored(&self) {
        let retained = self
            .service
            .recorded_context_binding(&support::actor(), &self.decide.run_id, "live.node.2")
            .expect("restored read")
            .expect("retained");
        assert_eq!(retained.binding, self.owner.0.lock().expect("bindings")[0]);
    }
}

#[test]
fn corrupt_record_identities_encoding_and_size_fail_closed() {
    let fixture = Fixture::new("integrity-record");
    for (path, replacement) in [
        ("/schema_version", json!("unsupported.history.v9")),
        ("/binding/workflow_run_id", json!("foreign.run")),
        ("/binding/definition_digest", json!("f".repeat(64))),
        ("/binding/node_execution_id", json!("foreign.node")),
        ("/command_id", json!("foreign.command")),
        ("/run_revision", json!(999)),
    ] {
        let mut changed: Value = serde_json::from_slice(&fixture.record).expect("record JSON");
        *changed.pointer_mut(path).expect("record field") = replacement;
        fixture.replace_record(&serde_json::to_vec(&changed).expect("encode"));
        fixture.assert_rejected_without_effects(path);
        fixture.replace_record(&fixture.record);
        fixture.assert_restored();
    }
    fixture.replace_record(b"{malformed");
    fixture.assert_rejected_without_effects("record JSON");
    fixture.replace_record(&fixture.record);
    fixture
        .connection
        .execute_batch("PRAGMA ignore_check_constraints = ON")
        .expect("simulate corrupt persisted store");
    fixture.replace_record(&vec![b' '; 16_385]);
    fixture.assert_rejected_without_effects("oversized record");
    fixture.replace_record(&fixture.record);
    fixture.assert_restored();
}

#[test]
fn corrupt_command_response_cannot_validate_historical_owner_evidence() {
    let fixture = Fixture::new("integrity-response");
    for (path, replacement) in [
        ("/schema_version", json!("unsupported.management.v9")),
        ("/workflow_run_id", json!("foreign.run")),
        ("/command_id", json!("foreign.command")),
        ("/run_revision", json!(999)),
        ("/sequence", Value::Null),
    ] {
        let mut changed: Value = serde_json::from_slice(&fixture.response).expect("response JSON");
        *changed.pointer_mut(path).expect("response field") = replacement;
        fixture.replace_response(&serde_json::to_vec(&changed).expect("encode"));
        fixture.assert_rejected_without_effects(path);
        fixture.replace_response(&fixture.response);
        fixture.assert_restored();
    }
    fixture.replace_response(b"{malformed");
    fixture.assert_rejected_without_effects("response JSON");
    fixture.replace_response(&fixture.response);
    fixture.assert_restored();
    assert_changed_payload_conflict(&fixture);
}

fn assert_changed_payload_conflict(fixture: &Fixture) {
    let calls = fixture.factory.entries();
    let actor = support::actor();
    let original = fixture
        .service
        .command(&actor, fixture.decide.clone())
        .expect("exact replay");
    let mut changed = fixture.decide.clone();
    changed.kind = CommandKind::Cancel;
    assert_eq!(
        fixture
            .service
            .command(&actor, changed)
            .expect_err("payload conflict")
            .code,
        "command_conflict"
    );
    assert_eq!(
        fixture
            .service
            .command(&actor, fixture.decide.clone())
            .expect("still exact replay"),
        original
    );
    fixture.assert_restored();
    assert_eq!(fixture.factory.entries(), calls);
    assert_eq!(fixture.owner.0.lock().expect("bindings").len(), 1);
}
