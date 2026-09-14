// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

#[path = "support/memory_policy_owner.rs"]
mod fixture;
use fixture::*;
use sts2_harness::context_memory::{policy_owner::*, *};

fn assert_stale(fixture: &Fixture) {
    match fixture.owner.prepare_active(access(), preparation()) {
        Err(error) => assert_eq!(error, PolicyOwnerError::StaleReview),
        Ok(prepared) => panic!(
            "stale approval prepared binding {} with sources {:?}",
            prepared.binding.binding_id, prepared.realized.selection.selected_sources
        ),
    }
}

fn revalidation(fixture: &Fixture, id: &str) -> PolicyReview {
    let active = fixture.owner.active_binding(access()).unwrap().unwrap();
    let raw = fixture
        .owner
        .inspect_policy(access(), &active.target)
        .unwrap()
        .raw_bytes()
        .to_vec();
    fixture
        .owner
        .execute(
            access(),
            PolicyCommand::ProposeRevalidation {
                key: format!("propose-{id}"),
                review_id: id.to_owned(),
                source: active.target,
                target_raw: raw,
                expected_active_version: active.version,
            },
        )
        .unwrap();
    let review = fixture.owner.inspect_review(access(), id).unwrap();
    fixture.approve(&review);
    review
}

fn adopt_revalidation(fixture: &Fixture) {
    let review = revalidation(fixture, "fresh-review");
    fixture
        .owner
        .execute(access(), Fixture::adopt_command(&review))
        .unwrap();
}

#[test]
fn restoring_a_corpus_clone_cannot_reactivate_revoked_content() {
    let fixture = Fixture::new();
    fixture.adopt();
    let retained = fixture
        .authority
        .inspect(|state| Ok(state.corpus.clone()))
        .unwrap();
    let reference = entry("history", 1).reference();
    assert_eq!(
        fixture
            .owner
            .prepare_active(access(), preparation())
            .unwrap()
            .realized
            .selection
            .selected_sources,
        vec![reference.clone()]
    );
    fixture
        .authority
        .update(|state| {
            state.corpus.revoke(&[reference], "2026-09-14T12:00:00Z")?;
            Ok(())
        })
        .unwrap();
    assert_stale(&fixture);
    fixture
        .authority
        .update(|state| {
            state.corpus = retained;
            Ok(())
        })
        .unwrap();
    // The generation and revocation values now match the old approval again.
    assert_stale(&fixture);
}

#[test]
fn same_generation_different_corpus_requires_explicit_revalidation() {
    let fixture = Fixture::new();
    fixture.adopt();
    let replacement = entry("replacement", 1);
    let expected = replacement.reference();
    fixture
        .authority
        .update(|state| {
            let mut corpus = MemoryCorpus::with_limits(scope(), 16, 4096)?;
            corpus.admit(replacement)?;
            assert_eq!(corpus.generation(), state.corpus.generation());
            assert_eq!(corpus.revocation_epoch(), state.corpus.revocation_epoch());
            state.corpus = corpus;
            Ok(())
        })
        .unwrap();
    assert_stale(&fixture);
    adopt_revalidation(&fixture);
    let prepared = fixture
        .owner
        .prepare_active(access(), preparation())
        .unwrap();
    assert_eq!(prepared.binding.version, 2);
    assert_eq!(prepared.realized.selection.selected_sources, vec![expected]);
}

#[test]
fn phase2_revision_round_trip_cannot_restore_old_approval() {
    let fixture = Fixture::new();
    fixture.adopt();
    fixture
        .authority
        .update(|state| {
            state.phase2_revision_id = "revision-2".to_owned();
            Ok(())
        })
        .unwrap();
    assert_stale(&fixture);
    fixture
        .authority
        .update(|state| {
            state.phase2_revision_id = "revision-1".to_owned();
            Ok(())
        })
        .unwrap();
    assert_stale(&fixture);
}

#[test]
fn descriptor_round_trip_cannot_restore_old_approval() {
    let fixture = Fixture::new();
    fixture.adopt();
    let retained = fixture
        .authority
        .inspect(|state| Ok(state.capabilities.clone()))
        .unwrap();
    fixture
        .authority
        .update(|state| {
            state.capabilities.effective_limits.optional_byte_budget -= 1;
            state.capabilities.binding.descriptor_sha256 = state.capabilities.descriptor_digest();
            Ok(())
        })
        .unwrap();
    assert_stale(&fixture);
    fixture
        .authority
        .update(|state| {
            state.capabilities = retained;
            Ok(())
        })
        .unwrap();
    assert_stale(&fixture);
}

#[test]
fn maintenance_rejects_approval_of_a_stale_unapproved_review() {
    let fixture = Fixture::new();
    let review = fixture.propose();
    fixture.authority.update(|_| Ok(())).unwrap();
    assert_eq!(
        fixture.owner.execute(
            access(),
            PolicyCommand::Approve {
                key: "stale-approval".to_owned(),
                review_id: review.review_id,
                review_sha256: review.review_sha256,
            },
        ),
        Err(PolicyOwnerError::StaleReview)
    );
    assert_eq!(
        fixture.owner.lookup_receipt(access(), "stale-approval"),
        Err(PolicyOwnerError::Missing)
    );
}

#[test]
fn successful_noop_fences_pending_and_active_approvals_until_revalidation() {
    let fixture = Fixture::new();
    fixture.adopt();
    let pending = revalidation(&fixture, "pending-review");
    fixture.authority.update(|_| Ok(())).unwrap();
    assert_eq!(
        fixture
            .owner
            .execute(access(), Fixture::adopt_command(&pending)),
        Err(PolicyOwnerError::StaleReview)
    );
    assert_stale(&fixture);
    adopt_revalidation(&fixture);
    fixture
        .owner
        .prepare_active(access(), preparation())
        .unwrap();
}

#[test]
fn disabled_corpus_counter_reset_is_allowed_but_cannot_restore_old_approval() {
    let fixture = Fixture::new();
    fixture.adopt();
    let retained = fixture
        .authority
        .inspect(|state| Ok(state.corpus.clone()))
        .unwrap();
    fixture
        .authority
        .update(|state| {
            state.corpus.set_enabled(false);
            state.capabilities = state.corpus.capabilities();
            Ok(())
        })
        .unwrap();
    fixture
        .authority
        .inspect(|state| {
            assert!(!state.corpus.enabled());
            assert_eq!(state.corpus.generation(), 0);
            assert_eq!(state.corpus.revocation_epoch(), 0);
            Ok(())
        })
        .unwrap();
    assert_stale(&fixture);
    fixture
        .authority
        .update(|state| {
            state.corpus = retained;
            state.capabilities = state.corpus.capabilities();
            Ok(())
        })
        .unwrap();
    assert_stale(&fixture);
    adopt_revalidation(&fixture);
    fixture
        .owner
        .prepare_active(access(), preparation())
        .unwrap();
}

#[test]
fn failed_updates_publish_neither_mutation_nor_epoch_and_rollback_is_rejected() {
    let fixture = Fixture::new();
    fixture.adopt();
    assert_eq!(
        fixture.authority.update(|state| {
            state.corpus.set_enabled(false);
            Err(PolicyOwnerError::PersistenceFailure)
        }),
        Err(PolicyOwnerError::PersistenceFailure)
    );
    assert_eq!(
        fixture.authority.update(|state| {
            state.phase2_revision_id.clear();
            Ok(())
        }),
        Err(PolicyOwnerError::ScopeMismatch)
    );
    fixture
        .owner
        .prepare_active(access(), preparation())
        .unwrap();
    fixture
        .authority
        .update(|state| {
            state.owner_epoch = 10;
            Ok(())
        })
        .unwrap();
    assert_eq!(
        fixture.authority.update(|state| {
            state.owner_epoch = 9;
            state.phase2_revision_id = "revision-rollback".to_owned();
            Ok(())
        }),
        Err(PolicyOwnerError::StaleReview)
    );
    fixture
        .authority
        .inspect(|state| {
            assert_eq!(state.owner_epoch, 10);
            assert_eq!(state.phase2_revision_id, "revision-1");
            assert!(state.corpus.enabled());
            Ok(())
        })
        .unwrap();
}

#[test]
fn safe_integer_epoch_exhaustion_rejects_mutation_atomically() {
    let fixture = Fixture::new();
    fixture
        .authority
        .update(|state| {
            state.owner_epoch = 9_007_199_254_740_991;
            Ok(())
        })
        .unwrap();
    assert_eq!(
        fixture.authority.update(|state| {
            state.corpus.set_enabled(false);
            Ok(())
        }),
        Err(PolicyOwnerError::Capacity)
    );
    fixture
        .authority
        .inspect(|state| {
            assert_eq!(state.owner_epoch, 9_007_199_254_740_991);
            assert!(state.corpus.enabled());
            assert_eq!(state.corpus.generation(), 1);
            Ok(())
        })
        .unwrap();
}
