// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use super::*;

#[test]
fn suppressed_or_altered_private_write_cannot_succeed_and_rolls_back() {
    // Deliberately invoke only the private write helper under hostile SQL. The owner boundary
    // rejects these objects before executing them; this separately proves write verification.
    for trigger in [
        "CREATE TRIGGER fail_write BEFORE UPDATE ON policy_journal BEGIN SELECT RAISE(IGNORE); END",
        "CREATE TRIGGER fail_write AFTER UPDATE ON policy_journal
         BEGIN UPDATE policy_journal SET epoch=1 WHERE id=1; END",
        "CREATE TRIGGER fail_write AFTER UPDATE ON policy_journal
         BEGIN UPDATE policy_journal SET envelope=zeroblob(40) WHERE id=1; END",
    ] {
        let mut connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(SCHEMA).unwrap();
        let scope = MemoryScope::new("project", "run", "episode", "agent");
        let mut journal = PolicyJournal::empty(scope.clone());
        let original = encrypt(&[7; 32], &scope, &journal).unwrap();
        write_envelope(&connection, 1, &original).unwrap();
        connection.execute_batch(trigger).unwrap();
        journal.store_epoch = 2;
        let candidate = encrypt(&[7; 32], &scope, &journal).unwrap();
        let transaction = connection.transaction().unwrap();
        assert_eq!(
            write_envelope(&transaction, 2, &candidate),
            Err(PolicyOwnerError::PersistenceFailure)
        );
        transaction.rollback().unwrap();
        assert_eq!(read_envelope(&connection).unwrap(), (1, original));
    }
}

#[test]
fn schema_checks_reject_temporary_objects_and_oversized_utf8_ddl() {
    let connection = Connection::open_in_memory().unwrap();
    connection.execute_batch(SCHEMA).unwrap();
    connection
        .execute_batch("CREATE TEMP VIEW injected AS SELECT 1")
        .unwrap();
    assert_eq!(
        check_schema(&connection),
        Err(PolicyOwnerError::StoreIncompatible)
    );
    connection.execute_batch("DROP VIEW temp.injected").unwrap();
    check_schema(&connection).unwrap();
    connection
        .execute_batch("PRAGMA writable_schema=ON")
        .unwrap();
    // Character count is below the metadata limit, byte count is above it.
    let sql = format!(
        "CREATE TABLE policy_store_meta(version INTEGER NOT NULL) /*{}*/",
        "火".repeat(700)
    );
    assert!(sql.chars().count() < 2048 && sql.len() > 2048);
    connection
        .execute(
            "UPDATE sqlite_master SET sql=?1 WHERE name='policy_store_meta'",
            [&sql],
        )
        .unwrap();
    assert_eq!(
        check_schema(&connection),
        Err(PolicyOwnerError::StoreIncompatible)
    );
}
