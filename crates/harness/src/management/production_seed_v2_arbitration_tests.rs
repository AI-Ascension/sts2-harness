// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::{Arc, Barrier};

use super::reservation::{assert_store_failure, count};

use super::super::seed_v2_support::{
    CountingAuthority, OTHER_TOKEN, PrivateDirectory, client, derive_once_request, open_store,
    request_bytes, runtime_counts, start_live_server,
};
use crate::management::{
    CommandAcceptance, CommandKind, CommandOutcome, CommandParameters, CommandRequest,
    MANAGEMENT_SCHEMA_VERSION, SeededRunSubmissionResponseV2,
    StoreCommandApplication as StoredCommandApplication, WorkflowRunStatus, WorkflowStore,
};

struct TamperingAuthority {
    inner: crate::management::FileSeedDerivationKeyAuthority,
    database: std::path::PathBuf,
    tampered: std::sync::atomic::AtomicBool,
}

impl TamperingAuthority {
    fn open(path: &std::path::Path, database: &std::path::Path) -> Self {
        Self {
            inner: crate::management::FileSeedDerivationKeyAuthority::open(path)
                .expect("protected keyring"),
            database: database.to_owned(),
            tampered: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

impl crate::management::SeedDerivationKeyAuthority for TamperingAuthority {
    fn current_key(
        &self,
    ) -> Result<crate::management::SeedKeyHandle, crate::management::SeedKeyError> {
        self.inner.current_key()
    }

    fn key_for(
        &self,
        authority_id: &str,
        version: &str,
    ) -> Result<Option<crate::management::SeedKeyHandle>, crate::management::SeedKeyError> {
        let key = self.inner.key_for(authority_id, version)?;
        if key.is_some()
            && !self
                .tampered
                .swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            let connection = rusqlite::Connection::open(&self.database)
                .map_err(|_| crate::management::SeedKeyError::Unavailable)?;
            let changed = connection
                .execute(
                    "UPDATE management_seed_operations
                     SET configuration_digest = ?1 WHERE phase = 'key_pinned'",
                    ["0".repeat(64)],
                )
                .map_err(|_| crate::management::SeedKeyError::Unavailable)?;
            if changed != 1 {
                return Err(crate::management::SeedKeyError::Unavailable);
            }
        }
        Ok(key)
    }
}

#[cfg(target_os = "linux")]
#[test]
fn served_seed_v2_arbitrates_once_and_rejects_seed_bound_admission_changes_atomically() {
    let directory = PrivateDirectory::create();
    let database = directory.path().join("workflow.sqlite");
    let keyring = directory.keyring("keys-v1.conf", "key-1", &[("key-1", "11")]);
    let first_store = open_store(&database);
    let second_store = open_store(&database);
    let first_keys = Arc::new(CountingAuthority::open(&keyring));
    let second_keys = Arc::new(CountingAuthority::open(&keyring));
    let first_server = start_live_server(
        Arc::clone(&first_store),
        Arc::clone(&first_keys),
        "live.catalog.v1",
    );
    let second_server = start_live_server(
        Arc::clone(&second_store),
        Arc::clone(&second_keys),
        "live.catalog.v1",
    );
    let (request, _) = derive_once_request();
    let body = request_bytes(&request);
    let barrier = Arc::new(Barrier::new(2));

    let first_barrier = Arc::clone(&barrier);
    let first_body = body.clone();
    let first_client = client(&first_server);
    let first = std::thread::spawn(move || {
        first_barrier.wait();
        first_client.request_json("POST", "/v2/workflow-runs", Some(&first_body))
    });
    let second_barrier = Arc::clone(&barrier);
    let second_client = client(&second_server);
    let second = std::thread::spawn(move || {
        second_barrier.wait();
        second_client.request_json("POST", "/v2/workflow-runs", Some(&body))
    });
    let first = first
        .join()
        .expect("first request thread")
        .expect("first response");
    let second = second
        .join()
        .expect("second request thread")
        .expect("second response");
    assert_eq!(first.status, 200);
    assert_eq!(second.status, 200);
    let first: SeededRunSubmissionResponseV2 =
        serde_json::from_slice(&first.body).expect("first v2 response");
    let second: SeededRunSubmissionResponseV2 =
        serde_json::from_slice(&second.body).expect("second v2 response");
    assert_eq!(first.run.workflow_run_id, second.run.workflow_run_id);
    assert_eq!(
        first.seed_binding.operation_id,
        second.seed_binding.operation_id
    );
    assert_eq!(
        first.seed_binding.effective_seed,
        second.seed_binding.effective_seed
    );
    assert!(first.seed_binding.state == crate::management::SeedBindingStateV2::AwaitingHostContext);
    assert_eq!(first.run.status, WorkflowRunStatus::Created);
    assert_eq!(runtime_counts(&first_server.runtime_counters), (0, 0, 0, 0));
    assert_eq!(
        runtime_counts(&second_server.runtime_counters),
        (0, 0, 0, 0)
    );
    assert_eq!(
        first_store
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT count(*) FROM management_seed_bindings", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
        1,
        "one independently opened SQLite store wins durable arbitration"
    );

    let current_key_reads = first_keys.current_reads() + second_keys.current_reads();
    let other_actor =
        crate::management::ManagementClient::new(first_server.server.address(), OTHER_TOKEN)
            .unwrap()
            .request_json("POST", "/v2/workflow-runs", Some(&request_bytes(&request)))
            .unwrap();
    assert_eq!(other_actor.status, 409);
    let other_actor: serde_json::Value = serde_json::from_slice(&other_actor.body).unwrap();
    assert_eq!(other_actor["error"]["code"], "seed_submission_conflict");

    let mut changed_body = request.clone();
    changed_body.seed.mode = crate::management::SeedModeV2::Explicit;
    changed_body.seed.seed = Some("different-explicit-seed".to_owned());
    let changed_body = client(&first_server)
        .request_json(
            "POST",
            "/v2/workflow-runs",
            Some(&request_bytes(&changed_body)),
        )
        .unwrap();
    assert_eq!(changed_body.status, 409);
    let changed_body: serde_json::Value = serde_json::from_slice(&changed_body.body).unwrap();
    assert_eq!(changed_body["error"]["code"], "seed_submission_conflict");
    assert_eq!(
        first_keys.current_reads() + second_keys.current_reads(),
        current_key_reads,
        "actor and body conflicts are decided before current-key selection"
    );

    let stored = first_store
        .read_seed_binding(&first.run.workflow_run_id)
        .unwrap()
        .expect("durable seed tuple");
    assert_eq!(
        stored.record().admitted_configuration.catalog_revision,
        "live.catalog.v1"
    );
    assert_eq!(stored.record().configuration_digest.len(), 64);
    let command = CommandRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        command_id: "seed-binding-control-update".to_owned(),
        run_id: first.run.workflow_run_id.clone(),
        expected_revision: 1,
        actor_scope: super::super::seed_v2_support::SUBJECT.to_owned(),
        kind: CommandKind::Pause,
        parameters: CommandParameters::default(),
    };
    let command_digest = crate::management::digest_value(
        &serde_json::to_value(&command).expect("serialize command"),
    )
    .expect("command digest");
    assert!(matches!(
        first_store
            .accept_command(&command, &command_digest)
            .unwrap(),
        CommandAcceptance::New { .. }
    ));
    let snapshot_before = first_store
        .get_run(&first.run.workflow_run_id)
        .unwrap()
        .expect("stored run snapshot");
    let events_before = first_store
        .events(&first.run.workflow_run_id, 0, 100)
        .unwrap();
    let command_before = command_state(&first_store, &command);
    assert_eq!(command_before, (1, None));

    let mut changed_snapshot = snapshot_before.clone();
    changed_snapshot.run_revision = 2;
    changed_snapshot.status = WorkflowRunStatus::Paused;
    changed_snapshot
        .admission
        .as_mut()
        .unwrap()
        .catalog_revision = "attacker-replacement".to_owned();
    let refused = first_store.apply_command(
        &command,
        &command_digest,
        StoredCommandApplication {
            snapshot: changed_snapshot,
            outcome: CommandOutcome::Applied,
            reason_code: "test_admission_replacement".to_owned(),
        },
    );
    assert_eq!(refused.unwrap_err().code, "seed_admission_immutable");
    assert_eq!(
        first_store.get_run(&first.run.workflow_run_id).unwrap(),
        Some(snapshot_before),
        "refused application leaves the complete run snapshot unchanged"
    );
    assert_eq!(
        first_store
            .events(&first.run.workflow_run_id, 0, 100)
            .unwrap(),
        events_before,
        "refused application creates no event or history row"
    );
    assert_eq!(
        command_state(&first_store, &command),
        command_before,
        "refused application leaves in-flight state and response unchanged"
    );

    first_store
        .release_command(&command, &command_digest)
        .unwrap();
    assert!(matches!(
        first_store
            .accept_command(&command, &command_digest)
            .unwrap(),
        CommandAcceptance::Existing {
            application_in_flight: false,
            response: None,
            ..
        }
    ));
    let mut evolved_snapshot = first_store
        .get_run(&first.run.workflow_run_id)
        .unwrap()
        .expect("run before control update");
    evolved_snapshot.run_revision = 2;
    evolved_snapshot.status = WorkflowRunStatus::Paused;
    first_store
        .apply_command(
            &command,
            &command_digest,
            StoredCommandApplication {
                snapshot: evolved_snapshot,
                outcome: CommandOutcome::Applied,
                reason_code: "test_control_update".to_owned(),
            },
        )
        .expect("same-admission snapshot evolution");
    assert_eq!(
        first_store
            .read_seed_binding(&first.run.workflow_run_id)
            .unwrap()
            .unwrap()
            .record()
            .admitted_configuration
            .catalog_revision,
        "live.catalog.v1",
        "seed verification authority remains the original immutable tuple"
    );

    first_server.server.shutdown().expect("stop first server");
    second_server.server.shutdown().expect("stop second server");
    drop(first_store);
    drop(second_store);
    directory.cleanup();
}

fn command_state(
    store: &crate::management::SqliteWorkflowStore,
    command: &CommandRequest,
) -> (i64, Option<Vec<u8>>) {
    store
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT application_in_flight, response FROM management_commands
             WHERE workflow_run_id = ?1 AND command_id = ?2",
            [&command.run_id, &command.command_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
}

#[cfg(target_os = "linux")]
#[test]
fn tampered_operation_index_is_rejected_before_candidate_rows_commit() {
    let directory = PrivateDirectory::create();
    let database = directory.path().join("tampered-operation-index.sqlite");
    let keyring = directory.keyring("keys.conf", "key-1", &[("key-1", "11")]);
    let store = open_store(&database);
    let keys = Arc::new(TamperingAuthority::open(&keyring, &database));
    let server = start_live_server(Arc::clone(&store), Arc::clone(&keys), "catalog.v1");
    let (request, _) = derive_once_request();
    let response = client(&server)
        .request_json("POST", "/v2/workflow-runs", Some(&request_bytes(&request)))
        .expect("served reservation with concurrent index tamper");
    assert_store_failure(&response);
    assert!(keys.tampered.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(count(&store, "management_seed_operations"), 1);
    for table in [
        "management_submissions",
        "management_runs",
        "management_events",
        "management_seed_bindings",
    ] {
        assert_eq!(
            count(&store, table),
            0,
            "corrupt pin must not write {table}"
        );
    }
    assert_eq!(runtime_counts(&server.runtime_counters), (0, 0, 0, 0));
    let actor_digest =
        crate::management::seed_v2_crypto::actor_digest(super::super::seed_v2_support::SUBJECT)
            .expect("actor digest");
    let request_digest = crate::management::seed_v2_crypto::request_digest(
        &request,
        super::super::seed_v2_support::SUBJECT,
    )
    .expect("request digest");
    match store.lookup_seed_operation(&request.request_id, &actor_digest, &request_digest) {
        Err(error) => assert_eq!(error.code, "store_corrupt"),
        Ok(_) => panic!("mismatched indexed configuration digest must fail closed"),
    }
    server.server.shutdown().expect("stop management server");
    drop(store);
    directory.cleanup();
}
