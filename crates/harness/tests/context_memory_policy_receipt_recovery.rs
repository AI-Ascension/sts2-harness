// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

#[path = "support/memory_policy_owner.rs"]
mod fixture;
#[path = "support/memory_policy_receipts.rs"]
mod receipts;
use fixture::*;
use receipts::*;
use sts2_harness::context_memory::{policy_owner::*, *};

#[test]
fn historical_receipts_survive_lost_reply_restart_and_newer_adoption() {
    let history = History::pending_second();
    let first = history
        .fixture
        .owner
        .lookup_receipt(access(), "adoption-one")
        .unwrap();
    let command = history.second_adoption();
    let before = row(&history.fixture);
    history
        .fixture
        .owner
        .set_failpoint(Some(PolicyStoreFailpoint::BeforeCommit))
        .unwrap();
    assert_eq!(
        history.fixture.owner.execute(access(), command.clone()),
        Err(PolicyOwnerError::PersistenceFailure)
    );
    assert_eq!(row(&history.fixture), before);
    assert_eq!(
        history
            .fixture
            .owner
            .lookup_receipt(access(), "adoption-two"),
        Err(PolicyOwnerError::Missing)
    );
    history
        .fixture
        .owner
        .set_failpoint(Some(PolicyStoreFailpoint::AfterCommit))
        .unwrap();
    assert_eq!(
        history.fixture.owner.execute(access(), command.clone()),
        Err(PolicyOwnerError::LostReply)
    );
    let second = history
        .fixture
        .owner
        .lookup_receipt(access(), "adoption-two")
        .unwrap();
    assert_eq!(
        history.fixture.owner.execute(access(), command).unwrap(),
        second
    );
    assert_eq!(first.result_id, "policy-binding-1");
    assert_eq!(second.result_id, "policy-binding-2");
    assert_eq!(
        history
            .fixture
            .owner
            .execute(access(), history.commands[3].clone())
            .unwrap(),
        first
    );
    assert_eq!(
        history
            .fixture
            .owner
            .active_binding(access())
            .unwrap()
            .unwrap()
            .binding_id,
        second.result_id
    );

    let reopened = MemoryPolicyOwner::open(
        &history.fixture.path,
        [7; 32],
        history.fixture.authority.clone(),
        PolicyStoreConsent::SyntheticOnly,
    )
    .unwrap();
    assert_eq!(
        reopened.lookup_receipt(access(), "adoption-one").unwrap(),
        first
    );
    assert_eq!(
        reopened
            .execute(access(), history.commands[3].clone())
            .unwrap(),
        first
    );
    assert_eq!(
        reopened
            .active_binding(access())
            .unwrap()
            .unwrap()
            .binding_id,
        "policy-binding-2"
    );
    assert!(matches!(
        reopened.prepare_active(access(), preparation()),
        Err(PolicyOwnerError::StaleReview)
    ));
}

#[test]
fn repeated_exact_imports_allow_colon_ids_and_distinct_authorized_subjects() {
    let fixture = Fixture::new();
    fixture
        .authority
        .update(|state| {
            let mut grant = state.grants["policy-grant"].clone();
            grant.grant_id = "other-grant".to_owned();
            grant.subject = "other".to_owned();
            state.grants.insert(grant.grant_id.clone(), grant);
            state.owner_epoch += 1;
            Ok(())
        })
        .unwrap();
    let other = PolicyAccess {
        bearer: Some("other-token"),
        grant_id: "other-grant",
    };
    let mut original = policy(1, MEMORY_POLICY_SCHEMA_MAX_OPTIONAL_BYTES);
    original.policy_id = "saved:policy:2026".to_owned();
    let raw = bytes(&original);
    let command = PolicyCommand::Import {
        key: "same-key".to_owned(),
        raw: raw.clone(),
    };
    let first = fixture.owner.execute(access(), command.clone()).unwrap();
    let second = fixture.owner.execute(other, command.clone()).unwrap();
    fixture
        .owner
        .execute(
            access(),
            PolicyCommand::Import {
                key: "another-key".to_owned(),
                raw: raw.clone(),
            },
        )
        .unwrap();
    assert_eq!(first.result_id, "saved:policy:2026:1");
    assert_eq!(first.result_id, second.result_id);
    assert_eq!(first.subject, "operator");
    assert_eq!(second.subject, "other");
    let reopened = MemoryPolicyOwner::open(
        &fixture.path,
        [7; 32],
        fixture.authority.clone(),
        PolicyStoreConsent::SyntheticOnly,
    )
    .unwrap();
    assert_eq!(
        reopened.lookup_receipt(access(), "same-key").unwrap(),
        first
    );
    assert_eq!(
        reopened
            .execute(
                PolicyAccess {
                    bearer: Some("other-token"),
                    grant_id: "other-grant"
                },
                command,
            )
            .unwrap(),
        second
    );
    assert_eq!(
        reopened
            .inspect_policy(access(), &reference(&raw))
            .unwrap()
            .raw_bytes(),
        raw
    );
    assert!(reopened.active_binding(access()).unwrap().is_none());
}
