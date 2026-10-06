// SPDX-License-Identifier: MIT

fn mark_v1(path: &PathBuf) {
    let connection = rusqlite::Connection::open(path).expect("open schema marker");
    connection
        .execute_batch(
            "UPDATE context_control_meta SET value = 'ascension.context-control.sqlite.v1' WHERE key = 'schema';
             UPDATE context_control_meta SET value = '1' WHERE key = 'schema_version';
             DROP TABLE IF EXISTS context_control_owner_state;",
        )
        .expect("restore exact v1 metadata and table set");
}

fn assert_v1_marker_and_no_owner_state(path: &PathBuf) {
    let connection = rusqlite::Connection::open(path).expect("check schema after refusal");
    assert_eq!(
        connection
            .query_row(
                "SELECT value FROM context_control_meta WHERE key = 'schema'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("legacy marker"),
        "ascension.context-control.sqlite.v1"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT value FROM context_control_meta WHERE key = 'schema_version'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("legacy version"),
        "1"
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'context_control_owner_state'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("owner table remains absent"),
        0
    );
}

#[test]
fn schema_v1_migration_preserves_encrypted_journal_and_source_bytes() {
    let path = path("v1-preservation");
    let key = [14_u8; 32];
    let authority = authority();
    let mut store =
        ContextControlStore::create(&path, key, "run-migration", &authority, StoreMode::Enabled)
            .expect("create store");
    let document = ContextSourceDocument {
        draft: ContextDraft::new("draft-migration", "revision-1"),
        items: Default::default(),
    };
    let source_digest = context_source_digest(&document).expect("source digest");
    store
        .publish_context_source(&DurableContextSourceSnapshot {
            source_id: "source-migration".into(),
            version: 1,
            digest: source_digest.clone(),
            document: document.clone(),
        })
        .expect("publish old encrypted source");
    drop(store);

    let before = rusqlite::Connection::open(&path).expect("open before migration");
    let journal_before = before
        .query_row(
            "SELECT envelope, envelope_digest FROM context_control_journal WHERE run_id = ?1",
            ["run-migration"],
            |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?)),
        )
        .expect("read old journal envelope");
    let source_before = before
        .query_row(
            "SELECT envelope, envelope_digest FROM context_control_context_sources
             WHERE run_id = ?1 AND source_id = ?2 AND version = 1",
            rusqlite::params!["run-migration", "source-migration"],
            |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?)),
        )
        .expect("read old source envelope");
    drop(before);

    let downgrade = rusqlite::Connection::open(&path).expect("open schema marker");
    downgrade
        .execute_batch(
            "UPDATE context_control_meta SET value = 'ascension.context-control.sqlite.v1' WHERE key = 'schema';
             UPDATE context_control_meta SET value = '1' WHERE key = 'schema_version';
             DROP TABLE context_control_owner_state;",
        )
        .expect("construct exact v1 marker and table set");
    drop(downgrade);

    let migrated =
        ContextControlStore::open(&path, key, "run-migration").expect("migrate v1 store");
    let mut expected_boundary = authority.state().boundary.clone();
    expected_boundary.controller_epoch = expected_boundary
        .controller_epoch
        .checked_add(1)
        .expect("recovered owner epoch advances");
    let expected = ControlAuthority::new(
        expected_boundary,
        authority.state().active_revision_id.clone(),
    )
    .with_max_control_events(authority.max_control_events())
    .expect("retain the selected event bound");
    let recovered = migrated.load().expect("old journal AAD still works");
    assert_eq!(
        recovered.state().boundary.controller_epoch,
        authority.state().boundary.controller_epoch + 1,
        "recovery advances exactly one owner epoch to fence the previous handle"
    );
    assert_eq!(
        recovered, expected,
        "journal state is preserved across owner fencing"
    );
    assert_eq!(
        migrated
            .load_context_source("source-migration", 1, &source_digest)
            .expect("old source AAD still works")
            .expect("source remains present")
            .document,
        document
    );
    let after = rusqlite::Connection::open(&path).expect("open migrated database");
    let journal_after = after
        .query_row(
            "SELECT envelope, envelope_digest FROM context_control_journal WHERE run_id = ?1",
            ["run-migration"],
            |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?)),
        )
        .expect("read migrated journal envelope");
    let source_after = after
        .query_row(
            "SELECT envelope, envelope_digest FROM context_control_context_sources
             WHERE run_id = ?1 AND source_id = ?2 AND version = 1",
            rusqlite::params!["run-migration", "source-migration"],
            |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?)),
        )
        .expect("read migrated source envelope");
    assert_eq!(journal_after, journal_before);
    assert_eq!(source_after, source_before);
    cleanup(&path);
}

#[test]
fn schema_v1_migration_failure_rolls_back_ddl_and_version_marker() {
    let path = path("v1-rollback");
    let setup = rusqlite::Connection::open(&path).expect("open migration fixture");
    setup
        .execute_batch(
            "CREATE TABLE context_control_meta (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);
             INSERT INTO context_control_meta(key, value) VALUES ('schema', 'ascension.context-control.sqlite.v1');
             INSERT INTO context_control_meta(key, value) VALUES ('schema_version', '1');
             CREATE TRIGGER reject_owner_schema_update BEFORE UPDATE ON context_control_meta
             WHEN OLD.key = 'schema'
             BEGIN SELECT RAISE(ABORT, 'injected migration failure'); END;",
        )
        .expect("seed v1 store and rollback trigger");
    drop(setup);

    assert!(matches!(
        ContextControlStore::open(&path, [15_u8; 32], "run-migration"),
        Err(DurableControlStoreError::Sqlite)
    ));
    let check = rusqlite::Connection::open(&path).expect("reopen rolled back v1");
    assert_eq!(
        check
            .query_row(
                "SELECT value FROM context_control_meta WHERE key = 'schema_version'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("old schema version remains"),
        "1"
    );
    assert_eq!(
        check
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'context_control_owner_state'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("new table rolled back"),
        0
    );
    check
        .execute("DROP TRIGGER reject_owner_schema_update", [])
        .expect("remove failure trigger");
    drop(check);
    assert!(ContextControlStore::open(&path, [15_u8; 32], "run-migration").is_ok());
    cleanup(&path);
}

#[test]
fn owner_state_compare_exchange_is_encrypted_cas_and_failpoint_atomic() {
    let path = path("owner-state-cas");
    let mut store = ContextControlStore::create(
        &path,
        [16_u8; 32],
        "run-migration",
        &authority(),
        StoreMode::Enabled,
    )
    .expect("create store");
    let sentinel = b"OWNER-DRAFT-PLAINTEXT-7b64c";
    let first_state = br#"{"content":"OWNER-DRAFT-PLAINTEXT-7b64c"}"#;
    assert_eq!(
        store
            .compare_exchange_owner_context_state("owner.migration", 0, first_state)
            .expect("first owner state"),
        1
    );
    assert_eq!(
        store
            .load_owner_context_state("owner.migration")
            .expect("load owner state")
            .expect("state exists")
            .bytes,
        first_state
    );
    assert_eq!(
        store.compare_exchange_owner_context_state("owner.migration", 0, br#"{"version":2}"#,),
        Err(DurableControlStoreError::OwnerContextConflict)
    );
    store.set_failpoint(Some(DurableStoreFailpoint::BeforeCommit));
    assert_eq!(
        store.compare_exchange_owner_context_state("owner.migration", 1, br#"{"version":2}"#,),
        Err(DurableControlStoreError::Failpoint)
    );
    assert_eq!(
        store
            .load_owner_context_state("owner.migration")
            .expect("load after rollback")
            .expect("original remains")
            .record_version,
        1
    );
    let raw = rusqlite::Connection::open(&path).expect("open persisted owner envelope");
    let envelope = raw
        .query_row(
            "SELECT envelope FROM context_control_owner_state WHERE run_id = ?1",
            ["run-migration"],
            |row| row.get::<_, Vec<u8>>(0),
        )
        .expect("read encrypted owner envelope");
    assert!(
        !envelope
            .windows(sentinel.len())
            .any(|window| window == sentinel)
    );
    drop(raw);
    drop(store);
    rusqlite::Connection::open(&path)
        .expect("open for checkpoint")
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .expect("checkpoint encrypted owner state");
    for candidate in [path.clone(), path.with_extension("sqlite-wal")] {
        if let Ok(raw) = fs::read(candidate) {
            assert!(!raw.windows(sentinel.len()).any(|window| window == sentinel));
        }
    }
    cleanup(&path);
}

#[test]
fn schema_v1_wrong_key_and_corrupt_envelope_refuse_before_migration() {
    let wrong_key_path = path("wrong-key-v1");
    let key = [31_u8; 32];
    ContextControlStore::create(
        &wrong_key_path,
        key,
        "run-migration",
        &authority(),
        StoreMode::Enabled,
    )
    .expect("create encrypted v2 fixture");
    mark_v1(&wrong_key_path);
    let before = rusqlite::Connection::open(&wrong_key_path)
        .expect("open v1 journal")
        .query_row(
            "SELECT envelope, envelope_digest FROM context_control_journal WHERE run_id = ?1",
            ["run-migration"],
            |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?)),
        )
        .expect("capture encrypted v1 journal");
    assert!(matches!(
        ContextControlStore::open(&wrong_key_path, [32_u8; 32], "run-migration"),
        Err(DurableControlStoreError::AuthenticationFailed)
    ));
    assert_v1_marker_and_no_owner_state(&wrong_key_path);
    let after = rusqlite::Connection::open(&wrong_key_path)
        .expect("reopen v1 journal")
        .query_row(
            "SELECT envelope, envelope_digest FROM context_control_journal WHERE run_id = ?1",
            ["run-migration"],
            |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?)),
        )
        .expect("read untouched encrypted v1 journal");
    assert_eq!(after, before);
    ContextControlStore::open(&wrong_key_path, key, "run-migration")
        .expect("rightful key can migrate after rejected opener");
    cleanup(&wrong_key_path);

    let corrupt_path = path("corrupt-v1");
    ContextControlStore::create(
        &corrupt_path,
        key,
        "run-migration",
        &authority(),
        StoreMode::Enabled,
    )
    .expect("create second encrypted fixture");
    mark_v1(&corrupt_path);
    let corrupt = rusqlite::Connection::open(&corrupt_path).expect("open to corrupt envelope");
    corrupt
        .execute(
            "UPDATE context_control_journal SET envelope = ?1 WHERE run_id = ?2",
            rusqlite::params![vec![0_u8; 64], "run-migration"],
        )
        .expect("corrupt encrypted journal without updating digest");
    drop(corrupt);
    assert!(matches!(
        ContextControlStore::open(&corrupt_path, key, "run-migration"),
        Err(DurableControlStoreError::Corrupt)
    ));
    assert_v1_marker_and_no_owner_state(&corrupt_path);
    cleanup(&corrupt_path);
}

#[test]
fn concurrent_v1_openers_observe_one_atomic_migration() {
    let path = path("concurrent-v1");
    let key = [33_u8; 32];
    ContextControlStore::create(
        &path,
        key,
        "run-migration",
        &authority(),
        StoreMode::Enabled,
    )
    .expect("create legacy source");
    mark_v1(&path);

    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let handles = (0..2)
        .map(|_| {
            let path = path.clone();
            let barrier = std::sync::Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                ContextControlStore::open(&path, key, "run-migration")
                    .map(|store| store.load().map(|_| ()))
                    .and_then(|loaded| loaded)
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    for handle in handles {
        handle
            .join()
            .expect("migration thread did not panic")
            .expect("concurrent valid open succeeds after one migration");
    }
    let store = ContextControlStore::open(&path, key, "run-migration").expect("v2 remains open");
    assert_eq!(
        store
            .load()
            .expect("migrated journal")
            .state()
            .boundary
            .run_id,
        "run-migration"
    );
    cleanup(&path);
}
