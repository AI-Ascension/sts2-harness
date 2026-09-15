// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

#[path = "support/memory_policy_owner.rs"]
mod fixture;
use fixture::*;
use sts2_harness::context_memory::{policy_owner::*, *};

fn epoch(path: &std::path::Path) -> i64 {
    rusqlite::Connection::open(path)
        .unwrap()
        .query_row("SELECT epoch FROM policy_journal", [], |row| row.get(0))
        .unwrap()
}

#[test]
fn failure_before_commit_and_lost_reply_do_not_duplicate_activation() {
    let fixture = Fixture::new();
    let review = fixture.propose();
    fixture.approve(&review);
    fixture
        .owner
        .set_failpoint(Some(PolicyStoreFailpoint::BeforeCommit))
        .unwrap();
    assert_eq!(
        fixture
            .owner
            .execute(access(), Fixture::adopt_command(&review)),
        Err(PolicyOwnerError::PersistenceFailure)
    );
    assert!(fixture.owner.active_binding(access()).unwrap().is_none());
    assert_eq!(
        fixture.owner.lookup_receipt(access(), "adopt-review"),
        Err(PolicyOwnerError::Missing)
    );
    fixture
        .owner
        .set_failpoint(Some(PolicyStoreFailpoint::AfterCommit))
        .unwrap();
    assert_eq!(
        fixture
            .owner
            .execute(access(), Fixture::adopt_command(&review)),
        Err(PolicyOwnerError::LostReply)
    );
    let receipt = fixture
        .owner
        .lookup_receipt(access(), "adopt-review")
        .unwrap();
    assert_eq!(
        fixture
            .owner
            .execute(access(), Fixture::adopt_command(&review))
            .unwrap(),
        receipt
    );
    assert_eq!(
        fixture
            .owner
            .active_binding(access())
            .unwrap()
            .unwrap()
            .version,
        1
    );
    assert_eq!(
        fixture.owner.execute(
            access(),
            PolicyCommand::Adopt {
                key: "adopt-review".to_owned(),
                review_id: review.review_id.clone(),
                review_sha256: sha256_hex(b"different"),
            }
        ),
        Err(PolicyOwnerError::Conflict)
    );
    assert!(
        fixture
            .owner
            .execute(
                access(),
                PolicyCommand::Adopt {
                    key: "second-adoption".to_owned(),
                    review_id: review.review_id,
                    review_sha256: review.review_sha256,
                }
            )
            .is_err()
    );
    assert_eq!(
        fixture
            .owner
            .prepare_active(access(), preparation())
            .unwrap()
            .binding
            .version,
        1
    );
}

#[test]
fn wrong_key_or_corrupt_history_cannot_claim_ownership() {
    let fixture = Fixture::new();
    fixture.adopt();
    let original_epoch = epoch(&fixture.path);
    assert!(matches!(
        MemoryPolicyOwner::open(
            &fixture.path,
            [8; 32],
            fixture.authority.clone(),
            PolicyStoreConsent::SyntheticOnly
        ),
        Err(PolicyOwnerError::Corrupt)
    ));
    assert_eq!(epoch(&fixture.path), original_epoch);
    fixture
        .owner
        .prepare_active(access(), preparation())
        .unwrap();
    let connection = rusqlite::Connection::open(&fixture.path).unwrap();
    let envelope: Vec<u8> = connection
        .query_row("SELECT envelope FROM policy_journal", [], |row| row.get(0))
        .unwrap();
    let mut corrupt = envelope.clone();
    let last = corrupt.len() - 1;
    corrupt[last] ^= 1;
    connection
        .execute("UPDATE policy_journal SET envelope=?1", [&corrupt])
        .unwrap();
    assert!(matches!(
        MemoryPolicyOwner::open(
            &fixture.path,
            [7; 32],
            fixture.authority.clone(),
            PolicyStoreConsent::SyntheticOnly
        ),
        Err(PolicyOwnerError::Corrupt)
    ));
    assert_eq!(epoch(&fixture.path), original_epoch);
    connection
        .execute("UPDATE policy_journal SET envelope=?1", [&envelope])
        .unwrap();
    fixture
        .owner
        .prepare_active(access(), preparation())
        .unwrap();
}

#[test]
fn schema_page_file_and_blob_bounds_fail_before_claiming_epoch() {
    let fixture = Fixture::new();
    let connection = rusqlite::Connection::open(&fixture.path).unwrap();
    connection
        .execute("UPDATE policy_store_meta SET version=2", [])
        .unwrap();
    assert!(matches!(
        MemoryPolicyOwner::open(
            &fixture.path,
            [7; 32],
            fixture.authority.clone(),
            PolicyStoreConsent::SyntheticOnly
        ),
        Err(PolicyOwnerError::StoreIncompatible)
    ));
    assert_eq!(epoch(&fixture.path), 1);
    connection
        .execute("UPDATE policy_store_meta SET version=1", [])
        .unwrap();
    connection
        .execute_batch("PRAGMA ignore_check_constraints=ON")
        .unwrap();
    connection
        .execute(
            "UPDATE policy_journal SET envelope=zeroblob(?1)",
            [MAX_POLICY_JOURNAL_BYTES as i64 + 41],
        )
        .unwrap();
    assert!(matches!(
        MemoryPolicyOwner::open(
            &fixture.path,
            [7; 32],
            fixture.authority.clone(),
            PolicyStoreConsent::SyntheticOnly
        ),
        Err(PolicyOwnerError::Capacity)
    ));
    assert_eq!(epoch(&fixture.path), 1);
    let oversized = fixture.directory.join("oversized.sqlite");
    let file = std::fs::File::create(&oversized).unwrap();
    file.set_len(MAX_POLICY_DATABASE_BYTES + 1).unwrap();
    assert!(matches!(
        MemoryPolicyOwner::open(
            &oversized,
            [7; 32],
            fixture.authority.clone(),
            PolicyStoreConsent::SyntheticOnly
        ),
        Err(PolicyOwnerError::Capacity)
    ));
    let wrong_page = fixture.directory.join("wrong-page.sqlite");
    let page_connection = rusqlite::Connection::open(&wrong_page).unwrap();
    page_connection
        .execute_batch("PRAGMA page_size=8192; CREATE TABLE unrelated(id INTEGER)")
        .unwrap();
    assert!(matches!(
        MemoryPolicyOwner::open(
            &wrong_page,
            [7; 32],
            fixture.authority.clone(),
            PolicyStoreConsent::SyntheticOnly
        ),
        Err(PolicyOwnerError::Capacity)
    ));
}

#[test]
fn bounded_history_refuses_capacity_without_eviction_or_plaintext() {
    let fixture = Fixture::new();
    for version in 1..=MAX_POLICY_VERSIONS as u64 {
        fixture
            .owner
            .execute(
                access(),
                PolicyCommand::Import {
                    key: format!("import-{version}"),
                    raw: bytes(&policy(version, 9000)),
                },
            )
            .unwrap();
    }
    assert_eq!(
        fixture.owner.execute(
            access(),
            PolicyCommand::Import {
                key: "over-count".to_owned(),
                raw: bytes(&policy(MAX_POLICY_VERSIONS as u64 + 1, 9000)),
            }
        ),
        Err(PolicyOwnerError::Capacity)
    );
    let original = bytes(&policy(1, 9000));
    assert_eq!(
        fixture
            .owner
            .inspect_policy(access(), &reference(&original))
            .unwrap()
            .raw_bytes(),
        original
    );
    assert_eq!(
        fixture.owner.execute(
            access(),
            PolicyCommand::Import {
                key: "over-bytes".to_owned(),
                raw: vec![b' '; MAX_POLICY_BYTES + 1],
            }
        ),
        Err(PolicyOwnerError::Capacity)
    );
    let persisted = std::fs::read(&fixture.path).unwrap();
    for needle in [
        "saved-policy",
        "saved-\\u0070olicy",
        "synthetic-owner-token",
        "optional_byte_budget",
    ] {
        assert!(
            !persisted
                .windows(needle.len())
                .any(|window| window == needle.as_bytes())
        );
    }
}

#[test]
fn same_key_recovers_after_restart_without_reexecution() {
    let fixture = Fixture::new();
    let review = fixture.propose();
    fixture.approve(&review);
    fixture
        .owner
        .set_failpoint(Some(PolicyStoreFailpoint::AfterCommit))
        .unwrap();
    assert_eq!(
        fixture
            .owner
            .execute(access(), Fixture::adopt_command(&review)),
        Err(PolicyOwnerError::LostReply)
    );
    let reopened = MemoryPolicyOwner::open(
        &fixture.path,
        [7; 32],
        fixture.authority.clone(),
        PolicyStoreConsent::SyntheticOnly,
    )
    .unwrap();
    let receipt = reopened.lookup_receipt(access(), "adopt-review").unwrap();
    assert_eq!(
        reopened
            .execute(access(), Fixture::adopt_command(&review))
            .unwrap(),
        receipt
    );
    assert_eq!(
        reopened.active_binding(access()).unwrap().unwrap().version,
        1
    );
    assert!(matches!(
        reopened.prepare_active(access(), preparation()),
        Err(PolicyOwnerError::StaleReview)
    ));
}
