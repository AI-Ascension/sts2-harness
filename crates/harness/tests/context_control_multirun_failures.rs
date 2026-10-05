#[test]
fn corrupt_foreign_run_is_not_attested_by_another_runs_migration() {
    let path = path("foreign-corruption");
    let key_a = [51_u8; 32];
    let key_b = [52_u8; 32];
    ContextControlStore::create(
        &path,
        key_a,
        "run-a",
        &authority("run-a"),
        StoreMode::Enabled,
    )
    .expect("create A");
    ContextControlStore::create(
        &path,
        key_b,
        "run-b",
        &authority("run-b"),
        StoreMode::Enabled,
    )
    .expect("create B");
    mark_v1(&path);
    let damaged = rusqlite::Connection::open(&path).expect("damage only B ciphertext");
    damaged
        .execute(
            "UPDATE context_control_journal SET envelope = ?1 WHERE run_id = ?2",
            rusqlite::params![vec![0_u8; 64], "run-b"],
        )
        .expect("corrupt foreign ciphertext without its digest");
    drop(damaged);

    ContextControlStore::open(&path, key_a, "run-a")
        .expect("A migration authenticates only A and preserves B bytes");
    let store_b = ContextControlStore::open(&path, key_b, "run-b").expect("open B v2 handle");
    assert_eq!(
        store_b.load().expect_err("B detects its own corrupt journal"),
        DurableControlStoreError::Corrupt
    );
    drop(store_b);
    cleanup(&path);
}

#[test]
fn corrupt_source_or_lifetime_refuses_v1_migration_without_schema_writes() {
    for table in [
        "context_control_context_sources",
        "context_control_lifetime",
    ] {
        let path = path(table);
        let key = [61_u8; 32];
        let mut store = ContextControlStore::create(
            &path,
            key,
            "run-corrupt-secondary",
            &authority("run-corrupt-secondary"),
            StoreMode::Enabled,
        )
        .expect("create v2 fixture");
        publish_source(&mut store, "run-corrupt-secondary");
        store
            .persist_lifetime(&ContextLifetimeLedger::new())
            .expect("persist lifetime");
        drop(store);
        mark_v1(&path);
        let damaged = rusqlite::Connection::open(&path).expect("open secondary envelope");
        damaged
            .execute(
                &format!("UPDATE {table} SET envelope = ?1 WHERE run_id = ?2"),
                rusqlite::params![vec![0_u8; 64], "run-corrupt-secondary"],
            )
            .expect("corrupt selected secondary envelope");
        drop(damaged);
        let rows_after_injected_damage = encrypted_rows(&path, "run-corrupt-secondary");

        assert!(matches!(
            ContextControlStore::open(&path, key, "run-corrupt-secondary"),
            Err(DurableControlStoreError::Corrupt)
        ));
        assert_v1_marker_and_no_owner_table(&path);
        assert_eq!(
            encrypted_rows(&path, "run-corrupt-secondary"),
            rows_after_injected_damage
        );
        cleanup(&path);
    }
}
