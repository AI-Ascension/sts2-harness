// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

#[path = "support/memory_policy_owner.rs"]
mod fixture;
use fixture::*;
use sts2_harness::context_memory::{policy_owner::*, *};

#[test]
fn exact_source_target_history_and_actual_active_policy_preparation() {
    let fixture = Fixture::new();
    let review = fixture.propose();
    let source = fixture
        .owner
        .inspect_policy(access(), &review.source)
        .unwrap();
    assert_eq!(
        source.raw_bytes(),
        bytes(&policy(1, MEMORY_POLICY_SCHEMA_MAX_OPTIONAL_BYTES))
    );
    assert!(
        fixture
            .owner
            .prepare_active(access(), preparation())
            .is_err()
    );
    fixture.approve(&review);
    assert!(fixture.owner.active_binding(access()).unwrap().is_none());
    fixture
        .owner
        .execute(access(), Fixture::adopt_command(&review))
        .unwrap();
    let prepared = fixture
        .owner
        .prepare_active(access(), preparation())
        .unwrap();
    assert_eq!(prepared.binding.target, review.target);
    assert_eq!(prepared.realized.selection.policy_version, 2);
    assert_eq!(prepared.realized.selection.selected_sources.len(), 1);
    assert_eq!(prepared.realized.selection.whole_tokens, None);
    assert_eq!(prepared.realized.retrieval.inference_calls, 0);
    let target = fixture
        .owner
        .inspect_policy(access(), &review.target)
        .unwrap();
    assert_eq!(target.raw_bytes(), bytes(&policy(2, MAX_OPTIONAL_BYTES)));
    assert_ne!(target.execution_sha256, target.reference.raw_sha256);
    assert_eq!(
        target.execution_sha256,
        sha256_hex(serde_json::to_vec(&policy(2, MAX_OPTIONAL_BYTES)).unwrap())
    );
}

#[test]
fn authentication_scope_full_policy_validation_and_target_approval_are_required() {
    let fixture = Fixture::new();
    let raw = bytes(&policy(1, 9000));
    for access in [
        PolicyAccess {
            bearer: None,
            grant_id: "policy-grant",
        },
        PolicyAccess {
            bearer: Some("wrong"),
            grant_id: "policy-grant",
        },
        PolicyAccess {
            bearer: Some("other-token"),
            grant_id: "policy-grant",
        },
        PolicyAccess {
            bearer: Some("synthetic-owner-token"),
            grant_id: "invented",
        },
    ] {
        assert!(
            fixture
                .owner
                .execute(
                    access,
                    PolicyCommand::Import {
                        key: "denied".to_owned(),
                        raw: raw.clone()
                    }
                )
                .is_err()
        );
    }
    let mut foreign = policy(1, 9000);
    foreign.scope.agent_id = "other".to_owned();
    assert_eq!(
        fixture.owner.execute(
            access(),
            PolicyCommand::Import {
                key: "foreign".to_owned(),
                raw: bytes(&foreign),
            }
        ),
        Err(PolicyOwnerError::ScopeMismatch)
    );
    let source = fixture.import();
    for (index, mut target) in [
        policy(2, 0),
        policy(2, MAX_OPTIONAL_BYTES + 1),
        policy(2, MAX_OPTIONAL_BYTES),
    ]
    .into_iter()
    .enumerate()
    {
        if index == 2 {
            target.approved_summary_catalog.push(MemoryRef {
                entry_id: "missing".to_owned(),
                version: 1,
                sha256: sha256_hex(b"missing"),
            });
        }
        assert!(
            fixture
                .owner
                .execute(
                    access(),
                    PolicyCommand::ProposeMigration {
                        key: format!("invalid-{index}"),
                        review_id: format!("invalid-{index}"),
                        source: reference(&source),
                        target_raw: bytes(&target),
                        expected_active_version: None,
                    }
                )
                .is_err()
        );
    }
    let review = fixture.propose();
    assert_eq!(
        fixture
            .owner
            .execute(access(), Fixture::adopt_command(&review)),
        Err(PolicyOwnerError::PermissionDenied)
    );
    assert_eq!(
        fixture.owner.execute(
            access(),
            PolicyCommand::Approve {
                key: "wrong-target".to_owned(),
                review_id: review.review_id,
                review_sha256: sha256_hex(b"another target"),
            }
        ),
        Err(PolicyOwnerError::StaleReview)
    );
    assert!(fixture.owner.active_binding(access()).unwrap().is_none());
}

#[test]
fn same_version_different_bytes_and_implicit_downgrade_conflict() {
    let fixture = Fixture::new();
    let review = fixture.adopt();
    let mut raw = bytes(&policy(2, MAX_OPTIONAL_BYTES));
    raw.push(b'\n');
    assert_eq!(
        fixture.owner.execute(
            access(),
            PolicyCommand::ProposeRevalidation {
                key: "changed".to_owned(),
                review_id: "changed".to_owned(),
                source: review.target.clone(),
                target_raw: raw,
                expected_active_version: 1,
            }
        ),
        Err(PolicyOwnerError::Conflict)
    );
    assert_eq!(
        fixture.owner.execute(
            access(),
            PolicyCommand::ProposeRevalidation {
                key: "older".to_owned(),
                review_id: "older".to_owned(),
                source: review.target,
                target_raw: bytes(&policy(1, MAX_OPTIONAL_BYTES)),
                expected_active_version: 1,
            }
        ),
        Err(PolicyOwnerError::Conflict)
    );
}

#[test]
fn restart_requires_explicit_revalidation_and_preserves_original_bytes() {
    let fixture = Fixture::new();
    let original = fixture.adopt();
    let reopened = MemoryPolicyOwner::open(
        &fixture.path,
        [7; 32],
        fixture.authority.clone(),
        PolicyStoreConsent::SyntheticOnly,
    )
    .unwrap();
    assert_eq!(
        reopened
            .inspect_policy(access(), &original.source)
            .unwrap()
            .raw_bytes(),
        bytes(&policy(1, MEMORY_POLICY_SCHEMA_MAX_OPTIONAL_BYTES))
    );
    assert!(matches!(
        fixture.owner.prepare_active(access(), preparation()),
        Err(PolicyOwnerError::OwnerFenced)
    ));
    assert!(matches!(
        reopened.prepare_active(access(), preparation()),
        Err(PolicyOwnerError::StaleReview)
    ));
    reopened
        .execute(
            access(),
            PolicyCommand::ProposeRevalidation {
                key: "revalidate".to_owned(),
                review_id: "revalidate".to_owned(),
                source: original.target,
                target_raw: bytes(&policy(2, MAX_OPTIONAL_BYTES)),
                expected_active_version: 1,
            },
        )
        .unwrap();
    let review = reopened.inspect_review(access(), "revalidate").unwrap();
    assert_eq!(review.kind, ReviewKind::Revalidation);
    assert!(review.violations.is_empty());
    assert!(
        reopened
            .execute(access(), Fixture::adopt_command(&review))
            .is_err()
    );
    reopened
        .execute(
            access(),
            PolicyCommand::Approve {
                key: "reapprove".to_owned(),
                review_id: review.review_id.clone(),
                review_sha256: review.review_sha256.clone(),
            },
        )
        .unwrap();
    reopened
        .execute(access(), Fixture::adopt_command(&review))
        .unwrap();
    let prepared = reopened.prepare_active(access(), preparation()).unwrap();
    assert_eq!(prepared.binding.version, 2);
    assert_eq!(prepared.realized.selection.policy_version, 2);
}
