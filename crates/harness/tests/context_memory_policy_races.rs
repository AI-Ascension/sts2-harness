// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

#[path = "support/memory_policy_owner.rs"]
mod fixture;
use fixture::*;
use std::sync::{
    Arc, Barrier,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use sts2_harness::context_memory::{policy_owner::*, *};

#[test]
fn stale_owner_grant_revision_profile_and_corpus_fences_reject_adoption() {
    for change in 0..6 {
        let fixture = Fixture::new();
        let review = fixture.propose();
        fixture.approve(&review);
        fixture
            .authority
            .update(|state| {
                match change {
                    0 => {
                        let grant = state.grants.get_mut("policy-grant").unwrap();
                        grant.revoked = true;
                        grant.epoch += 1;
                    }
                    1 => {
                        state.phase2_revision_id = "revision-2".to_owned();
                        state.control_epoch += 1;
                    }
                    2 => {
                        state.plan_epoch += 1;
                    }
                    3 => {
                        state.owner_epoch += 1;
                    }
                    4 => {
                        state.capabilities.effective_limits.optional_byte_budget -= 1;
                        state.capabilities.binding.descriptor_sha256 =
                            state.capabilities.descriptor_digest();
                    }
                    _ => {
                        state.corpus.admit(entry("later", 2)).unwrap();
                    }
                }
                Ok(())
            })
            .unwrap();
        assert!(
            fixture
                .owner
                .execute(access(), Fixture::adopt_command(&review))
                .is_err()
        );
        // Trusted inspection is used only to restore read permission after the revoked-grant case.
        if change == 0 {
            fixture
                .authority
                .update(|state| {
                    let grant = state.grants.get_mut("policy-grant").unwrap();
                    grant.revoked = false;
                    grant.epoch += 1;
                    Ok(())
                })
                .unwrap();
        }
        assert!(fixture.owner.active_binding(access()).unwrap().is_none());
    }
}

struct ExpiringClock {
    armed: AtomicBool,
    calls: AtomicUsize,
}
impl PolicyClock for ExpiringClock {
    fn now_seconds(&self) -> u64 {
        if self.armed.load(Ordering::SeqCst) && self.calls.fetch_add(1, Ordering::SeqCst) >= 3 {
            1000
        } else {
            100
        }
    }
    fn now_timestamp(&self) -> String {
        "2026-09-14T12:00:00Z".to_owned()
    }
}

#[test]
fn clock_expiry_at_precommit_rolls_back_active_binding_and_receipt() {
    let fixture = Fixture::new();
    let review = fixture.propose();
    fixture.approve(&review);
    let clock = Arc::new(ExpiringClock {
        armed: AtomicBool::new(false),
        calls: AtomicUsize::new(0),
    });
    // Keep the same store handle while replacing the clock by creating this test's owner first.
    let authority = authority(clock.clone());
    let path = fixture.directory.join("expiry.sqlite");
    let owner =
        MemoryPolicyOwner::open(path, [7; 32], authority, PolicyStoreConsent::SyntheticOnly)
            .unwrap();
    let raw = bytes(&policy(1, 9000));
    owner
        .execute(
            access(),
            PolicyCommand::Import {
                key: "import".to_owned(),
                raw: raw.clone(),
            },
        )
        .unwrap();
    owner
        .execute(
            access(),
            PolicyCommand::ProposeMigration {
                key: "propose".to_owned(),
                review_id: "review".to_owned(),
                source: reference(&raw),
                target_raw: bytes(&policy(2, MAX_OPTIONAL_BYTES)),
                expected_active_version: None,
            },
        )
        .unwrap();
    let review = owner.inspect_review(access(), "review").unwrap();
    owner
        .execute(
            access(),
            PolicyCommand::Approve {
                key: "approve".to_owned(),
                review_id: review.review_id.clone(),
                review_sha256: review.review_sha256.clone(),
            },
        )
        .unwrap();
    clock.armed.store(true, Ordering::SeqCst);
    assert_eq!(
        owner.execute(access(), Fixture::adopt_command(&review)),
        Err(PolicyOwnerError::GrantRevoked)
    );
    clock.armed.store(false, Ordering::SeqCst);
    assert!(owner.active_binding(access()).unwrap().is_none());
    assert_eq!(
        owner.lookup_receipt(access(), "adopt-review"),
        Err(PolicyOwnerError::Missing)
    );
}

struct GateClock {
    armed: AtomicBool,
    entered: Barrier,
    release: Barrier,
}
impl PolicyClock for GateClock {
    fn now_seconds(&self) -> u64 {
        if self.armed.swap(false, Ordering::SeqCst) {
            self.entered.wait();
            self.release.wait();
        }
        100
    }
    fn now_timestamp(&self) -> String {
        "2026-09-14T12:00:00Z".to_owned()
    }
}

#[test]
fn authority_lease_orders_revocation_after_atomic_adoption_and_fences_prepare() {
    let fixture = Fixture::new();
    let clock = Arc::new(GateClock {
        armed: AtomicBool::new(false),
        entered: Barrier::new(2),
        release: Barrier::new(2),
    });
    let authority = authority(clock.clone());
    let owner = Arc::new(
        MemoryPolicyOwner::open(
            fixture.directory.join("lease.sqlite"),
            [7; 32],
            authority.clone(),
            PolicyStoreConsent::SyntheticOnly,
        )
        .unwrap(),
    );
    let raw = bytes(&policy(1, 9000));
    owner
        .execute(
            access(),
            PolicyCommand::Import {
                key: "import".to_owned(),
                raw: raw.clone(),
            },
        )
        .unwrap();
    owner
        .execute(
            access(),
            PolicyCommand::ProposeMigration {
                key: "propose".to_owned(),
                review_id: "review".to_owned(),
                source: reference(&raw),
                target_raw: bytes(&policy(2, MAX_OPTIONAL_BYTES)),
                expected_active_version: None,
            },
        )
        .unwrap();
    let review = owner.inspect_review(access(), "review").unwrap();
    owner
        .execute(
            access(),
            PolicyCommand::Approve {
                key: "approve".to_owned(),
                review_id: review.review_id.clone(),
                review_sha256: review.review_sha256.clone(),
            },
        )
        .unwrap();
    clock.armed.store(true, Ordering::SeqCst);
    let adopting = {
        let owner = owner.clone();
        std::thread::spawn(move || owner.execute(access(), Fixture::adopt_command(&review)))
    };
    clock.entered.wait();
    let attempted = Arc::new(Barrier::new(2));
    let revoked = Arc::new(AtomicBool::new(false));
    let revoking = {
        let attempted = attempted.clone();
        let revoked = revoked.clone();
        std::thread::spawn(move || {
            attempted.wait();
            authority
                .update(|state| {
                    let grant = state.grants.get_mut("policy-grant").unwrap();
                    grant.revoked = true;
                    grant.epoch += 1;
                    revoked.store(true, Ordering::SeqCst);
                    Ok(())
                })
                .unwrap();
        })
    };
    attempted.wait();
    assert!(!revoked.load(Ordering::SeqCst));
    clock.release.wait();
    adopting.join().unwrap().unwrap();
    revoking.join().unwrap();
    assert!(matches!(
        owner.prepare_active(access(), preparation()),
        Err(PolicyOwnerError::GrantRevoked)
    ));
}

#[test]
fn fresh_selector_does_not_bypass_revoked_approval_grant() {
    let fixture = Fixture::new();
    fixture
        .authority
        .update(|state| {
            let mut selector = state.grants.get("policy-grant").unwrap().clone();
            selector.grant_id = "selector".to_owned();
            selector.permissions = std::collections::BTreeSet::from([PolicyPermission::Select]);
            state.grants.insert("selector".to_owned(), selector);
            state.owner_epoch += 1;
            Ok(())
        })
        .unwrap();
    fixture.adopt();
    fixture
        .owner
        .prepare_active(
            PolicyAccess {
                bearer: Some("synthetic-owner-token"),
                grant_id: "selector",
            },
            preparation(),
        )
        .unwrap();
    fixture
        .authority
        .update(|state| {
            let original = state.grants.get_mut("policy-grant").unwrap();
            original.revoked = true;
            original.epoch += 1;
            Ok(())
        })
        .unwrap();
    assert!(matches!(
        fixture.owner.prepare_active(
            PolicyAccess {
                bearer: Some("synthetic-owner-token"),
                grant_id: "selector",
            },
            preparation()
        ),
        Err(PolicyOwnerError::GrantRevoked)
    ));
}
