// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn checkpoint_catalog_bytes_survive_reopen_and_reconstruction() {
    let mut store = ExecutionStore::open_in_memory().expect("store opens");
    let old = lineage("attempt-raw", "trajectory-raw");
    store
        .start_episode(&old, &fingerprint())
        .expect("episode starts");
    let catalog_raw = br#"[ {"action_id":"combat.end-turn","action":{"kind":"end_turn"}} ]"#;
    let checkpoint = Checkpoint::new_with_catalog(
        old.clone(),
        2,
        "state-raw",
        7,
        fingerprint(),
        vec![b'{', b'}'],
        result_digest(catalog_raw),
        catalog_raw.to_vec(),
    )
    .expect("raw checkpoint is valid");
    store
        .save_checkpoint(&checkpoint)
        .expect("checkpoint saves");
    assert_eq!(
        store
            .last_checkpoint("episode-1")
            .expect("checkpoint reads")
            .expect("checkpoint exists")
            .catalog_raw,
        Some(catalog_raw.to_vec())
    );

    let next = lineage("attempt-raw-next", "trajectory-raw-next");
    store
        .reconstruct_attempt(&checkpoint, &next, &fingerprint(), "raw-prefix")
        .expect("reconstruction starts");
    assert_eq!(
        store
            .last_checkpoint("episode-1")
            .expect("reconstructed checkpoint reads")
            .expect("reconstructed checkpoint exists")
            .catalog_raw,
        Some(catalog_raw.to_vec())
    );
}

#[test]
fn checkpoint_reads_reject_oversized_and_malformed_catalog_bytes() {
    let database = path("oversized-checkpoint-catalog");
    let current = lineage(
        "attempt-checkpoint-hostile",
        "trajectory-checkpoint-hostile",
    );
    let catalog_raw = br#"[]"#;
    let checkpoint = Checkpoint::new_with_catalog(
        current.clone(),
        0,
        "state-hostile",
        1,
        fingerprint(),
        vec![b'{', b'}'],
        result_digest(catalog_raw),
        catalog_raw.to_vec(),
    )
    .expect("raw checkpoint is valid");
    let mut store =
        ExecutionStore::open(ExecutionStoreConfig::new(&database)).expect("store opens");
    store
        .start_episode(&current, &fingerprint())
        .expect("episode starts");
    store
        .save_checkpoint(&checkpoint)
        .expect("checkpoint saves");
    store.close().expect("store closes");
    drop(store);

    let connection = rusqlite::Connection::open(&database).expect("database opens for rewrite");
    let oversized = vec![b'x'; sts2_harness::MAX_CATALOG_BYTES + 1];
    connection
        .execute(
            "UPDATE checkpoints SET legal_actions_raw = ?1 WHERE episode_id = 'episode-1'",
            rusqlite::params![oversized],
        )
        .expect("oversized checkpoint row updates");
    drop(connection);
    let store = ExecutionStore::open(ExecutionStoreConfig::new(&database)).expect("state reopens");
    assert!(matches!(
        store.last_checkpoint("episode-1"),
        Err(sts2_harness::ExecutionStoreError::Corrupt)
    ));
    drop(store);
    remove_database(&database);

    let database = path("malformed-checkpoint-catalog");
    let mut store =
        ExecutionStore::open(ExecutionStoreConfig::new(&database)).expect("store opens");
    store
        .start_episode(&current, &fingerprint())
        .expect("episode starts");
    store
        .save_checkpoint(&checkpoint)
        .expect("checkpoint saves");
    store.close().expect("store closes");
    drop(store);
    let connection = rusqlite::Connection::open(&database).expect("database opens for rewrite");
    let malformed = br#"[}"#;
    connection
        .execute(
            "UPDATE checkpoints SET legal_actions_raw = ?1, legal_actions_digest = ?2
             WHERE episode_id = 'episode-1'",
            rusqlite::params![malformed.as_slice(), result_digest(malformed)],
        )
        .expect("malformed checkpoint row updates");
    drop(connection);
    let store = ExecutionStore::open(ExecutionStoreConfig::new(&database)).expect("state reopens");
    assert!(matches!(
        store.last_checkpoint("episode-1"),
        Err(sts2_harness::ExecutionStoreError::Corrupt)
    ));
    drop(store);
    remove_database(&database);
}
