// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use super::super::seed_v2_support::{
    CountingAuthority, PrivateDirectory, client, derive_once_request, open_store, request_bytes,
    runtime_counts, start_live_server,
};
use crate::management::{
    SeedBindingStateV2, SeedOperationLookup, SeededRunSubmissionResponseV2, WorkflowStore,
};

fn arm_trigger(store: &crate::management::SqliteWorkflowStore, sql: &str) {
    store
        .connection
        .lock()
        .expect("SQLite lock")
        .execute_batch(sql)
        .expect("install scoped SQLite fault trigger");
}

pub(super) fn count(store: &crate::management::SqliteWorkflowStore, table: &str) -> i64 {
    assert!(matches!(
        table,
        "management_seed_operations"
            | "management_submissions"
            | "management_runs"
            | "management_events"
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

pub(super) fn assert_store_failure(response: &crate::management::ClientResponse) {
    assert_eq!(
        response.status, 500,
        "injected SQLite write must fail as store error"
    );
    let body: serde_json::Value = serde_json::from_slice(&response.body).expect("error body");
    assert_eq!(body["error"]["class"], "store");
}

#[cfg(target_os = "linux")]
#[test]
fn reservation_commit_failure_selects_no_seed_and_rotated_retry_pins_new_key() {
    let directory = PrivateDirectory::create();
    let database = directory.path().join("reserve-before-commit.sqlite");
    let key_v1 = directory.keyring("keys-v1.conf", "key-1", &[("key-1", "11")]);
    let first_store = open_store(&database);
    arm_trigger(
        &first_store,
        "CREATE TRIGGER fail_seed_operation BEFORE INSERT ON management_seed_operations
         BEGIN SELECT RAISE(ABORT, 'injected reservation failure'); END;",
    );
    let first_keys = Arc::new(CountingAuthority::open(&key_v1));
    let first_server = start_live_server(
        Arc::clone(&first_store),
        Arc::clone(&first_keys),
        "live.catalog.v1",
    );
    let (request, _) = derive_once_request();
    let response = client(&first_server)
        .request_json("POST", "/v2/workflow-runs", Some(&request_bytes(&request)))
        .expect("served failed reservation response");
    assert_store_failure(&response);
    assert_eq!(first_keys.current_reads(), 1);
    assert_eq!(runtime_counts(&first_server.runtime_counters), (0, 0, 0, 0));
    for table in [
        "management_seed_operations",
        "management_submissions",
        "management_runs",
        "management_events",
        "management_seed_bindings",
    ] {
        assert_eq!(
            count(&first_store, table),
            0,
            "failed reservation leaves {table} empty"
        );
    }
    first_server
        .server
        .shutdown()
        .expect("stop first management server");
    arm_trigger(&first_store, "DROP TRIGGER fail_seed_operation;");
    drop(first_store);

    let rotated = directory.keyring("keys-v2.conf", "key-2", &[("key-1", "11"), ("key-2", "22")]);
    let second_store = open_store(&database);
    let second_keys = Arc::new(CountingAuthority::open(&rotated));
    let second_server = start_live_server(
        Arc::clone(&second_store),
        Arc::clone(&second_keys),
        "live.catalog.v2",
    );
    let response = client(&second_server)
        .request_json("POST", "/v2/workflow-runs", Some(&request_bytes(&request)))
        .expect("served retry after no reservation was committed");
    assert_eq!(response.status, 200);
    let response: SeededRunSubmissionResponseV2 =
        serde_json::from_slice(&response.body).expect("seeded response");
    assert_eq!(response.seed_binding.key_version.as_deref(), Some("key-2"));
    assert!(response.seed_binding.state == SeedBindingStateV2::AwaitingHostContext);
    assert_eq!(second_keys.current_reads(), 1);
    assert_eq!(count(&second_store, "management_seed_operations"), 1);
    assert_eq!(count(&second_store, "management_submissions"), 1);
    assert_eq!(count(&second_store, "management_runs"), 1);
    assert_eq!(count(&second_store, "management_seed_bindings"), 1);
    assert_eq!(
        runtime_counts(&second_server.runtime_counters),
        (0, 0, 0, 0)
    );
    second_server.server.shutdown().expect("stop retry server");
    drop(second_store);
    directory.cleanup();
}

#[cfg(target_os = "linux")]
#[test]
fn candidate_transaction_failure_keeps_key_pin_and_retry_uses_historical_key() -> Result<(), String>
{
    let directory = PrivateDirectory::create();
    let database = directory.path().join("candidate-rollback.sqlite");
    let key_v1 = directory.keyring("keys-v1.conf", "key-1", &[("key-1", "11")]);
    let first_store = open_store(&database);
    arm_trigger(
        &first_store,
        "CREATE TRIGGER fail_seed_binding BEFORE INSERT ON management_seed_bindings
         BEGIN SELECT RAISE(ABORT, 'injected candidate failure'); END;",
    );
    let first_keys = Arc::new(CountingAuthority::open(&key_v1));
    let first_server = start_live_server(
        Arc::clone(&first_store),
        Arc::clone(&first_keys),
        "live.catalog.v1",
    );
    let (request, _) = derive_once_request();
    let response = client(&first_server)
        .request_json("POST", "/v2/workflow-runs", Some(&request_bytes(&request)))
        .expect("served failed candidate response");
    assert_store_failure(&response);
    assert_eq!(first_keys.current_reads(), 1);
    assert!(first_keys.pinned_reads() > 0);
    assert_eq!(runtime_counts(&first_server.runtime_counters), (0, 0, 0, 0));
    assert_eq!(count(&first_store, "management_seed_operations"), 1);
    for table in [
        "management_submissions",
        "management_runs",
        "management_events",
        "management_seed_bindings",
    ] {
        assert_eq!(
            count(&first_store, table),
            0,
            "candidate rollback leaves {table} empty"
        );
    }
    let actor_digest =
        crate::management::seed_v2_crypto::actor_digest(super::super::seed_v2_support::SUBJECT)
            .expect("actor digest");
    let request_digest = crate::management::seed_v2_crypto::request_digest(
        &request,
        super::super::seed_v2_support::SUBJECT,
    )
    .expect("request digest");
    let operation = match first_store
        .lookup_seed_operation(&request.request_id, &actor_digest, &request_digest)
        .expect("read committed reservation")
    {
        SeedOperationLookup::Prepared(operation) => operation,
        _ => {
            first_server
                .server
                .shutdown()
                .expect("stop management server after unexpected reservation state");
            drop(first_store);
            directory.cleanup();
            return Err("failed candidate leaves the key-pinned operation prepared".to_owned());
        }
    };
    let expected_candidate = {
        let key = crate::management::seed_v2_crypto::load_pinned_operation_key(
            operation.record(),
            Some(first_keys.as_ref()),
        )
        .expect("reload exact historical key");
        crate::management::seed_v2_crypto::derive_candidate_from_operation(
            &request,
            &crate::management::AuthContext::new(
                super::super::seed_v2_support::SUBJECT,
                ["workflow:*".to_owned()],
            )
            .expect("actor"),
            &request_digest,
            operation.record(),
            &key,
        )
        .expect("derive expected value from committed operation")
        .effective_seed
    };
    first_server
        .server
        .shutdown()
        .expect("stop first management server");
    arm_trigger(&first_store, "DROP TRIGGER fail_seed_binding;");
    drop(first_store);

    let rotated = directory.keyring("keys-v2.conf", "key-2", &[("key-1", "11"), ("key-2", "22")]);
    let reopened = open_store(&database);
    let retry_keys = Arc::new(CountingAuthority::open(&rotated));
    let retry_server = start_live_server(
        Arc::clone(&reopened),
        Arc::clone(&retry_keys),
        "rotated.catalog.must-not-be-read",
    );
    let response = client(&retry_server)
        .request_json("POST", "/v2/workflow-runs", Some(&request_bytes(&request)))
        .expect("served recovery of prepared operation");
    assert_eq!(response.status, 200);
    let response: SeededRunSubmissionResponseV2 =
        serde_json::from_slice(&response.body).expect("replayed response");
    assert_eq!(response.seed_binding.effective_seed, expected_candidate);
    assert_eq!(response.seed_binding.key_version.as_deref(), Some("key-1"));
    assert_eq!(retry_keys.current_reads(), 0);
    assert!(retry_keys.pinned_reads() > 0);
    assert_eq!(
        retry_server
            .catalog_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    assert_eq!(count(&reopened, "management_seed_operations"), 1);
    assert_eq!(count(&reopened, "management_submissions"), 1);
    assert_eq!(count(&reopened, "management_runs"), 1);
    assert_eq!(count(&reopened, "management_seed_bindings"), 1);
    assert_eq!(runtime_counts(&retry_server.runtime_counters), (0, 0, 0, 0));
    retry_server.server.shutdown().expect("stop retry server");
    drop(reopened);
    directory.cleanup();
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn missing_or_changed_pinned_key_fails_closed_without_current_key_fallback() {
    let directory = PrivateDirectory::create();
    let database = directory.path().join("historical-key.sqlite");
    let key_v1 = directory.keyring("keys-v1.conf", "key-1", &[("key-1", "11")]);
    let store = open_store(&database);
    arm_trigger(
        &store,
        "CREATE TRIGGER fail_seed_binding BEFORE INSERT ON management_seed_bindings
         BEGIN SELECT RAISE(ABORT, 'injected candidate failure'); END;",
    );
    let keys = Arc::new(CountingAuthority::open(&key_v1));
    let server = start_live_server(Arc::clone(&store), Arc::clone(&keys), "live.catalog.v1");
    let (request, _) = derive_once_request();
    let response = client(&server)
        .request_json("POST", "/v2/workflow-runs", Some(&request_bytes(&request)))
        .expect("first candidate attempt");
    assert_store_failure(&response);
    server.server.shutdown().expect("stop initial server");
    arm_trigger(&store, "DROP TRIGGER fail_seed_binding;");
    drop(store);

    let missing = directory.keyring("keys-missing.conf", "key-2", &[("key-2", "22")]);
    let missing_store = open_store(&database);
    let missing_keys = Arc::new(CountingAuthority::open(&missing));
    let missing_server = start_live_server(
        Arc::clone(&missing_store),
        Arc::clone(&missing_keys),
        "catalog-not-read",
    );
    let response = client(&missing_server)
        .request_json("POST", "/v2/workflow-runs", Some(&request_bytes(&request)))
        .expect("missing-key refusal response");
    assert_ne!(response.status, 200);
    assert_eq!(missing_keys.current_reads(), 0);
    assert_eq!(missing_keys.pinned_reads(), 1);
    assert_eq!(count(&missing_store, "management_seed_operations"), 1);
    assert_eq!(count(&missing_store, "management_seed_bindings"), 0);
    missing_server
        .server
        .shutdown()
        .expect("stop missing-key server");
    drop(missing_store);

    let changed = directory.keyring(
        "keys-changed.conf",
        "key-2",
        &[("key-1", "99"), ("key-2", "22")],
    );
    let changed_store = open_store(&database);
    let changed_keys = Arc::new(CountingAuthority::open(&changed));
    let changed_server = start_live_server(
        Arc::clone(&changed_store),
        Arc::clone(&changed_keys),
        "catalog-not-read",
    );
    let response = client(&changed_server)
        .request_json("POST", "/v2/workflow-runs", Some(&request_bytes(&request)))
        .expect("changed-key refusal response");
    assert_ne!(response.status, 200);
    assert_eq!(changed_keys.current_reads(), 0);
    assert_eq!(changed_keys.pinned_reads(), 1);
    assert_eq!(count(&changed_store, "management_seed_operations"), 1);
    assert_eq!(count(&changed_store, "management_seed_bindings"), 0);
    changed_server
        .server
        .shutdown()
        .expect("stop changed-key server");
    drop(changed_store);

    let restored = directory.keyring(
        "keys-restored.conf",
        "key-2",
        &[("key-1", "11"), ("key-2", "22")],
    );
    let restored_store = open_store(&database);
    let restored_keys = Arc::new(CountingAuthority::open(&restored));
    let restored_server = start_live_server(
        Arc::clone(&restored_store),
        Arc::clone(&restored_keys),
        "catalog-not-read",
    );
    let response = client(&restored_server)
        .request_json("POST", "/v2/workflow-runs", Some(&request_bytes(&request)))
        .expect("recovery after restoring exact pinned key");
    assert_eq!(response.status, 200);
    let response: SeededRunSubmissionResponseV2 =
        serde_json::from_slice(&response.body).expect("restored response");
    assert_eq!(response.seed_binding.key_version.as_deref(), Some("key-1"));
    assert_eq!(restored_keys.current_reads(), 0);
    assert!(restored_keys.pinned_reads() > 0);
    assert_eq!(count(&restored_store, "management_seed_operations"), 1);
    assert_eq!(count(&restored_store, "management_seed_bindings"), 1);
    restored_server
        .server
        .shutdown()
        .expect("stop restored-key server");
    drop(restored_store);
    directory.cleanup();
}
