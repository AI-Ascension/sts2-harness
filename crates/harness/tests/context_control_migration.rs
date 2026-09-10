// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::PathBuf;

use sts2_harness::{
    ContextBoundary, ContextControlStore, ControlAuthority, DurableControlStoreError,
    DurableStoreFailpoint, LegacyOpenError, StoreMode,
};

fn boundary() -> ContextBoundary {
    ContextBoundary {
        run_id: "run-migration".to_owned(),
        episode_id: "episode-migration".to_owned(),
        agent_id: "agent-migration".to_owned(),
        state_id: "state-migration".to_owned(),
        generation: 1,
        observation_sha256: "a".repeat(64),
        catalog_sha256: "b".repeat(64),
        adapter_revision: "adapter-migration".to_owned(),
        model_revision: "model-migration".to_owned(),
        configuration_sha256: "c".repeat(64),
        output_schema_sha256: "d".repeat(64),
        controller_epoch: 1,
        gate_epoch: 0,
        control_version: 0,
    }
}

fn authority() -> ControlAuthority {
    ControlAuthority::new(boundary(), "revision-1")
}

fn path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "ascension-context-control-{label}-{}.sqlite",
        uuid::Uuid::new_v4()
    ))
}

fn cleanup(path: &PathBuf) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(path.with_extension("sqlite-wal"));
    let _ = fs::remove_file(path.with_extension("sqlite-shm"));
}

#[test]
fn encrypted_journal_reopens_with_phase1_snapshots_and_outbox_facts() {
    let path = path("reopen");
    let key = [7_u8; 32];
    let mut authority = authority();
    let mut store =
        ContextControlStore::create(&path, key, "run-migration", &authority, StoreMode::Enabled)
            .expect("create durable store");
    store
        .copy_phase1_snapshot("snapshot-v1", b"phase1 snapshot bytes")
        .expect("copy phase1 snapshot");
    authority
        .request_pause("pause-migration", 0)
        .expect("pause");
    store
        .persist(&authority, StoreMode::Enabled)
        .expect("persist paused authority");
    let before = store.snapshot().expect("snapshot facts");
    assert_eq!(before.mode, StoreMode::Enabled);
    assert_eq!(before.phase1_snapshot_count, 1);
    assert_eq!(before.phase1_snapshot_digests.len(), 1);
    assert_eq!(before.outbox_event_count, 1);
    assert_eq!(before.outbox_event_digests.len(), 1);
    assert!(!before.journal_digest.is_empty());

    let raw = fs::read(&path).expect("read sqlite file");
    assert!(
        !raw.windows(b"pause-migration".len())
            .any(|window| window == b"pause-migration")
    );

    drop(store);
    let reopened = ContextControlStore::open(&path, key, "run-migration").expect("reopen");
    let recovered = reopened.load().expect("recover journal");
    assert!(recovered.state().pause_latched);
    assert_eq!(recovered.state().boundary.controller_epoch, 2);
    assert_eq!(reopened.mode().expect("mode"), StoreMode::Enabled);
    assert_eq!(
        reopened.snapshot().expect("snapshot").phase1_snapshot_count,
        1
    );
    cleanup(&path);
}

#[test]
fn wrong_key_and_invalid_key_fail_closed_without_plaintext_fallback() {
    let path = path("key");
    let authority = authority();
    ContextControlStore::create(
        &path,
        [9_u8; 32],
        "run-migration",
        &authority,
        StoreMode::Enabled,
    )
    .expect("create");
    let wrong =
        ContextControlStore::open(&path, [8_u8; 32], "run-migration").expect("open with wrong key");
    assert_eq!(
        wrong.load().expect_err("wrong key must fail"),
        DurableControlStoreError::AuthenticationFailed
    );
    assert!(matches!(
        ContextControlStore::open(&path, [0_u8; 32], "run-migration"),
        Err(DurableControlStoreError::InvalidKey)
    ));
    cleanup(&path);
}

#[test]
fn failed_commit_rolls_back_the_active_control_state() {
    let path = path("rollback");
    let key = [11_u8; 32];
    let authority = authority();
    let mut store =
        ContextControlStore::create(&path, key, "run-migration", &authority, StoreMode::Enabled)
            .expect("create");
    let mut paused = authority.clone();
    paused.request_pause("pause-failpoint", 0).expect("pause");
    store.set_failpoint(Some(DurableStoreFailpoint::BeforeCommit));
    assert_eq!(
        store
            .persist(&paused, StoreMode::Enabled)
            .expect_err("failpoint must abort transaction"),
        DurableControlStoreError::Failpoint
    );
    assert!(
        !store
            .load()
            .expect("old state remains")
            .state()
            .pause_latched
    );
    store.persist(&paused, StoreMode::Enabled).expect("retry");
    assert!(store.load().expect("new state").state().pause_latched);
    cleanup(&path);
}

#[test]
fn additive_schema_retry_repairs_a_rolled_back_migration() {
    let path = path("migration");
    {
        let connection = rusqlite::Connection::open(&path).expect("raw sqlite");
        connection
            .execute_batch(
                "CREATE TABLE context_control_meta (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);
                 INSERT INTO context_control_meta(key, value) VALUES ('schema', 'ascension.context-control.sqlite.v1');
                 INSERT INTO context_control_meta(key, value) VALUES ('schema_version', '1');",
            )
            .expect("partial migration marker");
    }
    let authority = authority();
    let store =
        ContextControlStore::open(&path, [13_u8; 32], "run-migration").expect("retry migration");
    assert_eq!(
        store
            .snapshot()
            .expect_err("journal is still absent before initialization"),
        DurableControlStoreError::Missing
    );
    drop(store);
    let store = ContextControlStore::create(
        &path,
        [13_u8; 32],
        "run-migration",
        &authority,
        StoreMode::Disabled,
    )
    .expect("initialize after migration retry");
    assert_eq!(store.mode().expect("mode"), StoreMode::Disabled);
    cleanup(&path);
}

#[test]
fn legacy_binary_refuses_management_active_and_allows_disabled_state() {
    let path = path("legacy");
    let key = [17_u8; 32];
    let authority = authority();
    let mut store =
        ContextControlStore::create(&path, key, "run-migration", &authority, StoreMode::Enabled)
            .expect("create");
    assert_eq!(
        ContextControlStore::legacy_open(&path),
        Err(LegacyOpenError::ManagementActive)
    );
    store
        .persist(&authority, StoreMode::Disabled)
        .expect("safe deactivation marker");
    assert_eq!(ContextControlStore::legacy_open(&path), Ok(()));
    cleanup(&path);
}

#[test]
fn phase1_snapshot_identity_is_immutable_and_backup_is_reopenable() {
    let database = path("backup");
    let backup = path("backup-copy");
    let authority = authority();
    let mut store = ContextControlStore::create(
        &database,
        [19_u8; 32],
        "run-migration",
        &authority,
        StoreMode::Disabled,
    )
    .expect("create");
    store
        .copy_phase1_snapshot("snapshot-v1", b"immutable phase1 bytes")
        .expect("snapshot");
    assert_eq!(
        store
            .copy_phase1_snapshot("snapshot-v1", b"changed bytes")
            .expect_err("snapshot overwrite must fail"),
        DurableControlStoreError::SnapshotConflict
    );
    store.backup(&backup).expect("backup");
    let reopened =
        ContextControlStore::open(&backup, [19_u8; 32], "run-migration").expect("open backup");
    assert_eq!(
        reopened
            .snapshot()
            .expect("backup facts")
            .phase1_snapshot_count,
        1
    );
    cleanup(&database);
    cleanup(&backup);
}

#[test]
fn incompatible_schema_and_tampered_envelope_fail_closed() {
    let incompatible = path("incompatible");
    {
        let connection = rusqlite::Connection::open(&incompatible).expect("raw sqlite");
        connection
            .execute_batch(
                "CREATE TABLE context_control_meta (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);
                 INSERT INTO context_control_meta(key, value) VALUES ('schema', 'ascension.context-control.sqlite.v1');
                 INSERT INTO context_control_meta(key, value) VALUES ('schema_version', '2');",
            )
            .expect("newer marker");
    }
    assert!(matches!(
        ContextControlStore::open(&incompatible, [23_u8; 32], "run-migration"),
        Err(DurableControlStoreError::Incompatible)
    ));
    cleanup(&incompatible);

    let tampered = path("tampered");
    let authority = authority();
    ContextControlStore::create(
        &tampered,
        [29_u8; 32],
        "run-migration",
        &authority,
        StoreMode::Enabled,
    )
    .expect("create");
    {
        let connection = rusqlite::Connection::open(&tampered).expect("tamper sqlite");
        connection
            .execute(
                "UPDATE context_control_journal SET envelope_digest = '00' WHERE run_id = ?1",
                ["run-migration"],
            )
            .expect("tamper digest");
    }
    let reopened = ContextControlStore::open(&tampered, [29_u8; 32], "run-migration")
        .expect("open tampered store");
    assert_eq!(
        reopened
            .load()
            .expect_err("tampered digest must fail closed"),
        DurableControlStoreError::Corrupt
    );
    cleanup(&tampered);
}
