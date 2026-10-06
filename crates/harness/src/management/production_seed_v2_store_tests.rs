// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::{Arc, Mutex};

use super::seed_v2_support::{
    CountingAuthority, PrivateDirectory, client, derive_once_request, open_store, request_bytes,
    runtime_counts, start_live_server,
};
use super::{Counters, duplicate_run_tests};
use crate::management::{
    AuthContext, FileWorkflowStore, MemoryWorkflowStore, RunReservation, SeedOperationLookup,
    SeedOperationRecord, WorkflowExecutionPort, WorkflowStore, live_run_id,
};

#[cfg(target_os = "linux")]
#[test]
fn valid_candidate_with_a_different_owner_configuration_writes_no_run_or_event_rows() {
    let directory = PrivateDirectory::create();
    let database = directory.path().join("mismatched-admission.sqlite");
    let key_path = directory.keyring("keys.conf", "key-1", &[("key-1", "11")]);
    let keys = CountingAuthority::open(&key_path);
    let store = open_store(&database);
    let (request, definition_digest) = duplicate_run_tests::admitted_request();
    let binding = request.admission.clone().expect("owner admission");
    let mut different_binding = binding.clone();
    different_binding.catalog_revision = "different-valid-catalog".to_owned();
    different_binding
        .validate()
        .expect("different tuple remains structurally valid");
    let (v2_request, _) = derive_once_request();
    let actor = AuthContext::new(super::seed_v2_support::SUBJECT, ["workflow:*".to_owned()])
        .expect("actor");
    let request_digest =
        crate::management::seed_v2_crypto::request_digest(&v2_request, &actor.subject)
            .expect("closed request digest");
    let run_id = live_run_id(&request, &definition_digest).expect("run identity");
    let operation = crate::management::seed_v2_crypto::prepare_seed_operation(
        &v2_request,
        &actor,
        &request_digest,
        &run_id,
        &different_binding,
        Some(&keys),
    )
    .expect("key pin bound to a different but valid owner configuration");
    let operation = match store
        .reserve_seed_operation(SeedOperationRecord::new(operation.clone()))
        .expect("persist seed operation")
    {
        SeedOperationLookup::Prepared(operation) => *operation,
        _ => panic!("new operation reservation must remain prepared"),
    };
    let key = crate::management::seed_v2_crypto::load_pinned_operation_key(
        operation.record(),
        Some(&keys),
    )
    .expect("load reserved key");
    let record = crate::management::seed_v2_crypto::derive_candidate_from_operation(
        &v2_request,
        &actor,
        &request_digest,
        operation.record(),
        &key,
    )
    .expect("candidate uses durable operation key pin");
    crate::management::seed_v2_crypto::validate_candidate_record(&record)
        .expect("candidate's own binding and digest agree");

    let counters = Arc::new(Mutex::new(Counters::default()));
    let port = duplicate_run_tests::live_port(&counters, &run_id);
    let store_trait: Arc<dyn WorkflowStore> = store.clone();
    let reservation = RunReservation::for_seed_candidate(
        Arc::clone(&store_trait),
        v2_request.request_id.clone(),
        request_digest,
        definition_digest.clone(),
        Some(binding),
        Some(operation),
        record,
    );
    let error = port
        .prepare_seed_candidate_with_reservation(
            &request,
            &actor,
            &definition_digest,
            request.admission.as_ref(),
            &reservation,
        )
        .expect_err("candidate must match the actual reserved snapshot admission");
    assert_eq!(error.code, "seed_binding_identity_conflict");
    assert_eq!(duplicate_run_tests::counts(&counters), (0, 0, 0, 0));
    assert!(store.get_run(&run_id).unwrap().is_none());
    let connection = store.connection.lock().unwrap();
    for table in [
        "management_submissions",
        "management_runs",
        "management_events",
        "management_seed_bindings",
    ] {
        let query = format!("SELECT count(*) FROM {table}");
        let count = connection
            .query_row(&query, [], |row| row.get::<_, i64>(0))
            .unwrap();
        assert_eq!(count, 0, "rejected candidate leaves {table} empty");
    }
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM management_seed_operations",
                [],
                |row| { row.get::<_, i64>(0) }
            )
            .unwrap(),
        1,
        "the pre-candidate key pin survives a rejected finalization"
    );
    drop(connection);
    drop(store_trait);
    drop(store);
    directory.cleanup();
}

#[test]
fn legacy_file_store_open_is_byte_preserving_and_refuses_seed_binding_capability() {
    let path = std::env::temp_dir().join(format!(
        "sts2-seed-v2-legacy-file-{}.json",
        uuid::Uuid::new_v4()
    ));
    let legacy = br#"{"schema_version":"ascension.management/v1","submissions":{},"runs":{}}"#;
    std::fs::write(&path, legacy).expect("write baseline v1 file store fixture");

    let store = FileWorkflowStore::open(&path).expect("open legacy file store");
    assert!(!store.supports_durable_seed_bindings());
    assert!(!store.supports_seed_operation_reservations());
    let digest = "a".repeat(64);
    assert!(matches!(
        store.lookup_seed_binding("legacy-request", &digest, &digest),
        Err(error) if error.code == "seed_binding_unavailable"
    ));
    assert!(matches!(
        store.lookup_seed_operation("legacy-request", &digest, &digest),
        Err(error) if error.code == "seed_operation_unavailable"
    ));
    drop(store);

    assert_eq!(
        std::fs::read(&path).expect("read legacy file after open"),
        legacy,
        "opening a legacy unseeded store must not rewrite or upgrade it"
    );
    std::fs::remove_file(&path).expect("remove legacy fixture");

    let memory = MemoryWorkflowStore::default();
    assert!(!memory.supports_seed_operation_reservations());
    assert!(matches!(
        memory.lookup_seed_operation("memory-request", &digest, &digest),
        Err(error) if error.code == "seed_operation_unavailable"
    ));
}

#[cfg(target_os = "linux")]
#[test]
fn operation_lookup_keeps_one_sqlite_snapshot_across_candidate_commit() {
    let directory = PrivateDirectory::create();
    let database = directory.path().join("lookup-snapshot.sqlite");
    let keyring = directory.keyring("keys.conf", "key-1", &[("key-1", "11")]);
    let first_store = open_store(&database);
    first_store
        .connection
        .lock()
        .expect("SQLite lock")
        .execute_batch(
            "CREATE TRIGGER fail_seed_binding BEFORE INSERT ON management_seed_bindings
             BEGIN SELECT RAISE(ABORT, 'leave a prepared key pin'); END;",
        )
        .expect("install candidate fault");
    let first_keys = Arc::new(CountingAuthority::open(&keyring));
    let first_server = start_live_server(
        Arc::clone(&first_store),
        Arc::clone(&first_keys),
        "catalog.initial",
    );
    let (request, _) = derive_once_request();
    let first_response = client(&first_server)
        .request_json("POST", "/v2/workflow-runs", Some(&request_bytes(&request)))
        .expect("first request leaves key-pinned operation");
    assert_eq!(first_response.status, 500);
    let error: serde_json::Value =
        serde_json::from_slice(&first_response.body).expect("store error response");
    assert_eq!(error["error"]["class"], "store");
    first_server
        .server
        .shutdown()
        .expect("stop first management server");
    drop(first_store);

    let actor_digest =
        crate::management::seed_v2_crypto::actor_digest(super::seed_v2_support::SUBJECT)
            .expect("actor digest");
    let request_digest = crate::management::seed_v2_crypto::request_digest(
        &request,
        super::seed_v2_support::SUBJECT,
    )
    .expect("request digest");
    let reader = open_store(&database);
    let writer = open_store(&database);
    writer
        .connection
        .lock()
        .expect("SQLite lock")
        .execute_batch("DROP TRIGGER fail_seed_binding;")
        .expect("clear candidate fault");
    let writer_keys = Arc::new(CountingAuthority::open(&keyring));
    let writer_server = start_live_server(
        Arc::clone(&writer),
        Arc::clone(&writer_keys),
        "catalog.must-not-affect-the-pinned-operation",
    );
    let mut reader_connection = reader.connection.lock().expect("reader SQLite lock");
    let transaction = reader_connection
        .transaction()
        .expect("begin a stable read snapshot");
    let phase = transaction
        .query_row(
            "SELECT phase FROM management_seed_operations WHERE request_id = ?1",
            [&request.request_id],
            |row| row.get::<_, String>(0),
        )
        .expect("pin prepared row in the reader snapshot");
    assert_eq!(phase, "key_pinned");

    let committed = client(&writer_server)
        .request_json("POST", "/v2/workflow-runs", Some(&request_bytes(&request)))
        .expect("writer commits candidate after reader snapshot begins");
    assert_eq!(committed.status, 200);

    let snapshot_result =
        crate::management::SqliteWorkflowStore::lookup_seed_operation_in_transaction_for_test(
            &transaction,
            &request.request_id,
            &actor_digest,
            &request_digest,
        )
        .expect("operation and related-row reads share the pinned snapshot");
    assert!(matches!(snapshot_result, SeedOperationLookup::Prepared(_)));
    transaction
        .commit()
        .expect("finish the stable read snapshot");
    drop(reader_connection);

    let current_result = reader
        .lookup_seed_operation(&request.request_id, &actor_digest, &request_digest)
        .expect("a new read transaction sees the completed candidate");
    assert!(matches!(
        current_result,
        SeedOperationLookup::CandidatePersisted(_)
    ));
    assert_eq!(
        writer
            .connection
            .lock()
            .expect("SQLite lock")
            .query_row(
                "SELECT count(*) FROM management_seed_operations",
                [],
                |row| { row.get::<_, i64>(0) }
            )
            .expect("operation count"),
        1
    );
    assert_eq!(
        runtime_counts(&writer_server.runtime_counters),
        (0, 0, 0, 0)
    );
    assert_eq!(
        writer_server
            .catalog_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    writer_server
        .server
        .shutdown()
        .expect("stop writer management server");
    drop(writer);
    drop(reader);
    directory.cleanup();
}

#[cfg(target_os = "linux")]
#[test]
fn candidate_operation_lookup_requires_matching_submission_and_run_rows() {
    let directory = PrivateDirectory::create();
    let database = directory.path().join("candidate-operation-indexes.sqlite");
    let keyring = directory.keyring("keys.conf", "key-1", &[("key-1", "11")]);
    let store = open_store(&database);
    let keys = Arc::new(CountingAuthority::open(&keyring));
    let server = start_live_server(Arc::clone(&store), Arc::clone(&keys), "catalog.v1");
    let (request, _) = derive_once_request();
    let response = client(&server)
        .request_json("POST", "/v2/workflow-runs", Some(&request_bytes(&request)))
        .expect("persist candidate for index checks");
    assert_eq!(response.status, 200);
    server
        .server
        .shutdown()
        .expect("stop management server before direct row checks");

    let actor_digest =
        crate::management::seed_v2_crypto::actor_digest(super::seed_v2_support::SUBJECT)
            .expect("actor digest");
    let request_digest = crate::management::seed_v2_crypto::request_digest(
        &request,
        super::seed_v2_support::SUBJECT,
    )
    .expect("request digest");
    let run_id = store
        .connection
        .lock()
        .expect("SQLite lock")
        .query_row(
            "SELECT workflow_run_id FROM management_seed_operations WHERE request_id = ?1",
            [&request.request_id],
            |row| row.get::<_, String>(0),
        )
        .expect("operation run identity");
    let wrong_digest = "f".repeat(64);
    assert_ne!(wrong_digest, request_digest);
    {
        let connection = store.connection.lock().expect("SQLite lock");
        connection
            .execute(
                "UPDATE management_submissions SET request_digest = ?1 WHERE request_id = ?2",
                rusqlite::params![wrong_digest, request.request_id],
            )
            .expect("tamper submission digest index");
    }
    assert_store_corrupt(&store, &request.request_id, &actor_digest, &request_digest);
    {
        let connection = store.connection.lock().expect("SQLite lock");
        connection
            .execute(
                "UPDATE management_submissions SET request_digest = ?1 WHERE request_id = ?2",
                rusqlite::params![request_digest, request.request_id],
            )
            .expect("restore submission digest index");
        connection
            .execute(
                "UPDATE management_runs SET request_id = ?1 WHERE workflow_run_id = ?2",
                rusqlite::params!["orphaned-request-index", run_id],
            )
            .expect("tamper run request index");
    }
    assert_store_corrupt(&store, &request.request_id, &actor_digest, &request_digest);
    {
        let connection = store.connection.lock().expect("SQLite lock");
        connection
            .execute(
                "UPDATE management_runs SET request_id = ?1 WHERE workflow_run_id = ?2",
                rusqlite::params![request.request_id, run_id],
            )
            .expect("restore run request index");
        connection
            .execute_batch("PRAGMA foreign_keys=OFF;")
            .expect("disable foreign keys for corruption fixture only");
        connection
            .execute(
                "DELETE FROM management_runs WHERE workflow_run_id = ?1",
                [&run_id],
            )
            .expect("create orphan candidate row fixture");
    }
    assert_store_corrupt(&store, &request.request_id, &actor_digest, &request_digest);
    assert_eq!(table_count(&store, "management_seed_operations"), 1);
    assert_eq!(table_count(&store, "management_submissions"), 1);
    assert_eq!(table_count(&store, "management_runs"), 0);
    assert_eq!(table_count(&store, "management_seed_bindings"), 1);
    drop(store);
    directory.cleanup();
}

fn assert_store_corrupt(
    store: &crate::management::SqliteWorkflowStore,
    request_id: &str,
    actor_digest: &str,
    request_digest: &str,
) {
    match store.lookup_seed_operation(request_id, actor_digest, request_digest) {
        Err(error) => assert_eq!(error.code, "store_corrupt"),
        Ok(_) => panic!("inconsistent persisted candidate must fail closed"),
    }
}

fn table_count(store: &crate::management::SqliteWorkflowStore, table: &str) -> i64 {
    assert!(matches!(
        table,
        "management_seed_operations"
            | "management_submissions"
            | "management_runs"
            | "management_seed_bindings"
    ));
    store
        .connection
        .lock()
        .expect("SQLite lock")
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("table count")
}
