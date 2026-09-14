// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

#[path = "support/memory_policy_owner.rs"]
mod fixture;
use fixture::*;
use rusqlite::Connection;
use sts2_harness::context_memory::{policy_owner::*, *};

fn stored(connection: &Connection) -> (i64, Vec<u8>) {
    connection
        .query_row("SELECT epoch, envelope FROM policy_journal", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap()
}

#[test]
fn unexpected_schema_objects_cannot_ack_adoption_or_claim_ownership() {
    for (install, remove) in [
        (
            "CREATE TRIGGER suppressed BEFORE UPDATE ON policy_journal BEGIN SELECT RAISE(IGNORE); END",
            "DROP TRIGGER suppressed",
        ),
        (
            "CREATE VIEW unexpected AS SELECT envelope FROM policy_journal",
            "DROP VIEW unexpected",
        ),
        (
            "CREATE INDEX unexpected ON policy_journal(epoch)",
            "DROP INDEX unexpected",
        ),
    ] {
        let fixture = Fixture::new();
        let review = fixture.propose();
        fixture.approve(&review);
        let connection = Connection::open(&fixture.path).unwrap();
        let before = stored(&connection);
        connection.execute_batch(install).unwrap();
        assert_eq!(
            fixture
                .owner
                .execute(access(), Fixture::adopt_command(&review)),
            Err(PolicyOwnerError::StoreIncompatible)
        );
        assert!(matches!(
            MemoryPolicyOwner::open(
                &fixture.path,
                [7; 32],
                fixture.authority.clone(),
                PolicyStoreConsent::SyntheticOnly,
            ),
            Err(PolicyOwnerError::StoreIncompatible)
        ));
        assert_eq!(stored(&connection), before);
        connection.execute_batch(remove).unwrap();
        assert!(fixture.owner.active_binding(access()).unwrap().is_none());
        assert_eq!(
            fixture.owner.lookup_receipt(access(), "adopt-review"),
            Err(PolicyOwnerError::Missing)
        );
        // Rejection did not consume the approval, alter history or fence this healthy owner.
        fixture
            .owner
            .execute(access(), Fixture::adopt_command(&review))
            .unwrap();
        assert_eq!(
            fixture
                .owner
                .active_binding(access())
                .unwrap()
                .unwrap()
                .version,
            1
        );
        fixture
            .owner
            .prepare_active(access(), preparation())
            .unwrap();
    }
}

#[test]
fn altered_table_or_metadata_is_rejected_without_touching_the_encrypted_row() {
    for sql in [
        "ALTER TABLE policy_journal ADD COLUMN surprise TEXT",
        "UPDATE policy_store_meta SET version=2",
        "UPDATE policy_store_meta SET version='unsupported'",
        "INSERT INTO policy_store_meta VALUES(1)",
        "ALTER TABLE policy_store_meta RENAME TO old_meta;
         CREATE TABLE policy_store_meta(version INTEGER);
         INSERT INTO policy_store_meta VALUES(1); DROP TABLE old_meta",
        "ALTER TABLE policy_journal RENAME TO old_journal;
         CREATE TABLE policy_journal(id INTEGER PRIMARY KEY, epoch INTEGER NOT NULL, envelope BLOB NOT NULL);
         INSERT INTO policy_journal SELECT * FROM old_journal; DROP TABLE old_journal",
    ] {
        let fixture = Fixture::new();
        fixture.import();
        let connection = Connection::open(&fixture.path).unwrap();
        let before = stored(&connection);
        connection.execute_batch(sql).unwrap();
        assert!(matches!(MemoryPolicyOwner::open(
            &fixture.path, [7; 32], fixture.authority.clone(), PolicyStoreConsent::SyntheticOnly,
        ), Err(PolicyOwnerError::StoreIncompatible)), "{sql}");
        assert_eq!(fixture.owner.execute(access(), PolicyCommand::Import {
            key: "invalid-schema".to_owned(), raw: bytes(&policy(2, 9000)),
        }), Err(PolicyOwnerError::StoreIncompatible), "{sql}");
        assert_eq!(stored(&connection), before);
    }
}

#[test]
fn historical_v1_ddl_remains_compatible() {
    let fixture = Fixture::new();
    let source = Connection::open(&fixture.path).unwrap();
    let original = stored(&source);
    let legacy = fixture.directory.join("legacy-v1.sqlite");
    let connection = Connection::open(&legacy).unwrap();
    // Exact emitted definitions from the original v1 store, including internal whitespace.
    connection
        .execute_batch(
            "CREATE TABLE policy_store_meta(version INTEGER NOT NULL);
         INSERT INTO policy_store_meta VALUES(1);
CREATE TABLE policy_journal(
 id INTEGER PRIMARY KEY CHECK(id=1), epoch INTEGER NOT NULL,
 envelope BLOB NOT NULL CHECK(length(envelope)<=16777256));",
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO policy_journal VALUES(1,?1,?2)",
            rusqlite::params![original.0, original.1],
        )
        .unwrap();
    let opened = MemoryPolicyOwner::open(
        &legacy,
        [7; 32],
        fixture.authority.clone(),
        PolicyStoreConsent::SyntheticOnly,
    )
    .unwrap();
    assert_eq!(stored(&connection).0, 2);
    opened
        .execute(
            access(),
            PolicyCommand::Import {
                key: "legacy-import".to_owned(),
                raw: bytes(&policy(1, 9000)),
            },
        )
        .unwrap();
    assert_eq!(
        opened
            .lookup_receipt(access(), "legacy-import")
            .unwrap()
            .sequence,
        1
    );
}
