#[test]
fn schema_v1_migration_preserves_control_receipt_envelope_and_original_aad() {
    let path = store_path();
    let initial_binding = binding();
    let initial = ControlAuthority::new(
        initial_binding.boundary.clone(),
        initial_binding.approved_revision_id.clone(),
    );
    let mut paused = initial.clone();
    let command = ContextControlCommand::Pause {
        idempotency_key: "migration-preserved-pause".into(),
        expected_control_version: initial_binding.boundary.control_version,
    };
    let outcome = paused
        .request_pause(
            "migration-preserved-pause",
            initial_binding.boundary.control_version,
        )
        .expect("pause transition");
    let expected_record = record(
        initial_binding.clone(),
        command.clone(),
        receipt(&initial_binding, &command, outcome, &paused),
    );
    let mut store = ContextControlStore::create(&path, KEY, RUN_ID, &initial, StoreMode::Enabled)
        .expect("create source schema");
    store
        .persist_with_owner_control_receipt(&paused, StoreMode::Enabled, &expected_record)
        .expect("persist encrypted receipt");
    drop(store);

    let before_connection = rusqlite::Connection::open(&path).expect("read v2 receipt");
    let before = before_connection
        .query_row(
            "SELECT owner_id, command_digest, idempotency_digest, envelope, envelope_digest
             FROM context_control_owner_receipts WHERE run_id = ?1",
            [RUN_ID],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .expect("read receipt envelope before migration");
    drop(before_connection);
    let legacy = rusqlite::Connection::open(&path).expect("set v1 marker");
    legacy
        .execute_batch(
            "UPDATE context_control_meta SET value = 'ascension.context-control.sqlite.v1' WHERE key = 'schema';
             UPDATE context_control_meta SET value = '1' WHERE key = 'schema_version';
             DROP TABLE context_control_owner_state;",
        )
        .expect("construct exact old schema marker");
    drop(legacy);

    let migrated = ContextControlStore::open(&path, KEY, RUN_ID).expect("migrate receipt store");
    assert_eq!(
        migrated
            .lookup_owner_control_receipt(OWNER_ID, ACTOR, &command)
            .expect("original v1 receipt AAD remains valid"),
        Some(expected_record)
    );
    let after_connection = rusqlite::Connection::open(&path).expect("read migrated receipt");
    let after = after_connection
        .query_row(
            "SELECT owner_id, command_digest, idempotency_digest, envelope, envelope_digest
             FROM context_control_owner_receipts WHERE run_id = ?1",
            [RUN_ID],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .expect("read receipt envelope after migration");
    assert_eq!(after, before);
    drop(after_connection);
    drop(migrated);
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(path.with_extension("sqlite-wal"));
    let _ = fs::remove_file(path.with_extension("sqlite-shm"));
}

#[test]
fn corrupt_v1_receipt_refuses_migration_without_changing_schema_marker() {
    let path = store_path();
    let initial_binding = binding();
    let initial = ControlAuthority::new(
        initial_binding.boundary.clone(),
        initial_binding.approved_revision_id.clone(),
    );
    let mut paused = initial.clone();
    let command = ContextControlCommand::Pause {
        idempotency_key: "migration-corrupt-pause".into(),
        expected_control_version: initial_binding.boundary.control_version,
    };
    let outcome = paused
        .request_pause(
            "migration-corrupt-pause",
            initial_binding.boundary.control_version,
        )
        .expect("pause transition");
    let receipt = receipt(&initial_binding, &command, outcome, &paused);
    let mut store = ContextControlStore::create(&path, KEY, RUN_ID, &initial, StoreMode::Enabled)
        .expect("create receipt store");
    store
        .persist_with_owner_control_receipt(
            &paused,
            StoreMode::Enabled,
            &record(initial_binding, command, receipt),
        )
        .expect("persist encrypted receipt");
    drop(store);
    rusqlite::Connection::open(&path)
        .expect("mark legacy schema")
        .execute_batch(
            "UPDATE context_control_meta SET value = 'ascension.context-control.sqlite.v1' WHERE key = 'schema';
             UPDATE context_control_meta SET value = '1' WHERE key = 'schema_version';
             DROP TABLE context_control_owner_state;",
        )
        .expect("construct v1 schema");
    rusqlite::Connection::open(&path)
        .expect("corrupt receipt envelope")
        .execute(
            "UPDATE context_control_owner_receipts SET envelope = ?1 WHERE run_id = ?2",
            rusqlite::params![vec![0_u8; 64], RUN_ID],
        )
        .expect("damage receipt without changing digest");
    let damaged_before = rusqlite::Connection::open(&path)
        .expect("capture damaged receipt")
        .query_row(
            "SELECT envelope, envelope_digest FROM context_control_owner_receipts WHERE run_id = ?1",
            [RUN_ID],
            |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?)),
        )
        .expect("damaged receipt bytes");

    assert!(matches!(
        ContextControlStore::open(&path, KEY, RUN_ID),
        Err(DurableControlStoreError::Corrupt)
    ));
    let check = rusqlite::Connection::open(&path).expect("inspect refused migration");
    assert_eq!(
        check
            .query_row(
                "SELECT value FROM context_control_meta WHERE key = 'schema_version'",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("schema version remains v1"),
        "1"
    );
    assert_eq!(
        check
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'context_control_owner_state'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .expect("owner table absent"),
        0
    );
    drop(check);
    let damaged_after = rusqlite::Connection::open(&path)
        .expect("verify damaged receipt remains untouched")
        .query_row(
            "SELECT envelope, envelope_digest FROM context_control_owner_receipts WHERE run_id = ?1",
            [RUN_ID],
            |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?)),
        )
        .expect("damaged receipt after refused migration");
    assert_eq!(damaged_after, damaged_before);
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(path.with_extension("sqlite-wal"));
    let _ = fs::remove_file(path.with_extension("sqlite-shm"));
}
