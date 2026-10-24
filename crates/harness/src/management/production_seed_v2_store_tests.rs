// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::{Arc, Mutex};

use super::seed_v2_support::{
    CountingAuthority, PrivateDirectory, derive_once_request, open_store,
};
use super::{Counters, duplicate_run_tests};
use crate::management::{
    AuthContext, FileWorkflowStore, RunReservation, WorkflowExecutionPort, WorkflowStore,
    live_run_id,
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
    let record = crate::management::seed_v2_crypto::candidate_record(
        &v2_request,
        &actor,
        &request_digest,
        &run_id,
        &different_binding,
        Some(&keys),
    )
    .expect("candidate bound to a different but valid owner configuration");
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
    let digest = "a".repeat(64);
    assert!(matches!(
        store.lookup_seed_binding("legacy-request", &digest, &digest),
        Err(error) if error.code == "seed_binding_unavailable"
    ));
    drop(store);

    assert_eq!(
        std::fs::read(&path).expect("read legacy file after open"),
        legacy,
        "opening a legacy unseeded store must not rewrite or upgrade it"
    );
    std::fs::remove_file(&path).expect("remove legacy fixture");
}
