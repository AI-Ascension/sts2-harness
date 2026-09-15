// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

#[path = "support/memory_policy_owner.rs"]
mod fixture;
use fixture::*;
use sts2_harness::context_memory::{policy_owner::*, *};

#[test]
fn oversized_auth_and_preparation_inputs_stop_at_the_entry_boundary() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use sts2_harness::management::{AuthContext, AuthError, Authenticator};
    struct CountingAuth(AtomicUsize);
    impl Authenticator for CountingAuth {
        fn authenticate(&self, _: Option<&str>) -> Result<AuthContext, AuthError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(AuthError::InvalidCredentials)
        }
    }
    let fixture = Fixture::new();
    let auth = Arc::new(CountingAuth(AtomicUsize::new(0)));
    let authority =
        Arc::new(MemoryPolicyAuthority::new(state(), auth.clone(), fixture.clock.clone()).unwrap());
    let owner = MemoryPolicyOwner::open(
        fixture.directory.join("auth-bounds.sqlite"),
        [7; 32],
        authority,
        PolicyStoreConsent::SyntheticOnly,
    )
    .unwrap();
    let oversized = "x".repeat(4097);
    assert_eq!(
        owner.execute(
            PolicyAccess {
                bearer: Some(&oversized),
                grant_id: "policy-grant"
            },
            PolicyCommand::Import {
                key: "import".to_owned(),
                raw: bytes(&policy(1, 9000))
            }
        ),
        Err(PolicyOwnerError::Unauthenticated)
    );
    assert_eq!(auth.0.load(Ordering::SeqCst), 0);
    let mut request = preparation();
    request
        .mandatory_bytes
        .resize(MAX_JOB_INPUT_BYTES + 1, b'x');
    assert!(matches!(
        owner.prepare_active(access(), request),
        Err(PolicyOwnerError::Capacity)
    ));
    assert_eq!(auth.0.load(Ordering::SeqCst), 0);
    assert!(owner.prepare_active(access(), preparation()).is_err());
    assert_eq!(auth.0.load(Ordering::SeqCst), 1);
}

#[test]
fn trusted_maintenance_cannot_rebind_an_open_store_to_another_scope() {
    let fixture = Fixture::new();
    let review = fixture.adopt();
    assert_eq!(
        fixture.authority.update(|state| {
            let mut other = scope();
            other.agent_id = "other-agent".to_owned();
            state.corpus = MemoryCorpus::with_limits(other.clone(), 16, 4096).unwrap();
            state.capabilities = state.corpus.capabilities();
            for grant in state.grants.values_mut() {
                grant.scope = other.clone();
            }
            state.owner_epoch += 1;
            Ok(())
        }),
        Err(PolicyOwnerError::ScopeMismatch)
    );
    assert_eq!(
        fixture
            .owner
            .inspect_policy(access(), &review.target)
            .unwrap()
            .raw_bytes(),
        bytes(&policy(2, MAX_OPTIONAL_BYTES))
    );
    fixture
        .owner
        .prepare_active(access(), preparation())
        .unwrap();
}

#[test]
fn schema_limits_relations_and_profile_limits_have_distinct_errors() {
    let fixture = Fixture::new();
    let source = fixture.import();
    let mut relation = policy(2, MAX_OPTIONAL_BYTES);
    relation.max_candidates = 1;
    relation.max_results = 2;
    for (index, target) in [policy(2, 0), policy(2, 65_537), relation]
        .into_iter()
        .enumerate()
    {
        assert_eq!(
            fixture.owner.execute(
                access(),
                PolicyCommand::ProposeMigration {
                    key: format!("invalid-{index}"),
                    review_id: format!("invalid-{index}"),
                    source: reference(&source),
                    target_raw: bytes(&target),
                    expected_active_version: None,
                }
            ),
            Err(PolicyOwnerError::SchemaInvalid)
        );
    }
    assert_eq!(
        fixture.owner.execute(
            access(),
            PolicyCommand::ProposeMigration {
                key: "profile-over".to_owned(),
                review_id: "profile-over".to_owned(),
                source: reference(&source),
                target_raw: bytes(&policy(2, MAX_OPTIONAL_BYTES + 1)),
                expected_active_version: None,
            }
        ),
        Err(PolicyOwnerError::Memory(
            MemoryError::CapabilityLimitExceeded {
                limit: "optional_byte_budget".to_owned(),
                requested: MAX_OPTIONAL_BYTES + 1,
                effective: MAX_OPTIONAL_BYTES,
            }
        ))
    );
    let mut duplicate = bytes(&policy(3, MAX_OPTIONAL_BYTES));
    let text = String::from_utf8(duplicate)
        .unwrap()
        .replace("\"version\": 3", "\"version\": 3, \"version\": 3");
    duplicate = text.into_bytes();
    assert_eq!(
        fixture.owner.execute(
            access(),
            PolicyCommand::Import {
                key: "duplicate-field".to_owned(),
                raw: duplicate,
            }
        ),
        Err(PolicyOwnerError::SchemaInvalid)
    );
}

#[test]
fn revalidation_can_publish_newer_target_for_new_revision_and_corpus() {
    let fixture = Fixture::new();
    let old = fixture.adopt();
    fixture
        .authority
        .update(|state| {
            state.phase2_revision_id = "revision-2".to_owned();
            state.control_epoch += 1;
            state.corpus.admit(entry("later", 2)).unwrap();
            Ok(())
        })
        .unwrap();
    let mut target = policy(3, MAX_OPTIONAL_BYTES);
    target.phase2_revision_id = Some("revision-2".to_owned());
    target.corpus_generation = 2;
    fixture
        .owner
        .execute(
            access(),
            PolicyCommand::ProposeRevalidation {
                key: "new-context".to_owned(),
                review_id: "new-context".to_owned(),
                source: old.target.clone(),
                target_raw: bytes(&target),
                expected_active_version: 1,
            },
        )
        .unwrap();
    let review = fixture
        .owner
        .inspect_review(access(), "new-context")
        .unwrap();
    fixture.approve(&review);
    fixture
        .owner
        .execute(access(), Fixture::adopt_command(&review))
        .unwrap();
    let mut request = preparation();
    request.query.corpus_generation = 2;
    let prepared = fixture.owner.prepare_active(access(), request).unwrap();
    assert_eq!(prepared.realized.selection.policy_version, 3);
    assert_eq!(prepared.realized.selection.phase2_revision_id, "revision-2");
    assert_eq!(
        fixture
            .owner
            .inspect_policy(access(), &old.target)
            .unwrap()
            .raw_bytes(),
        bytes(&policy(2, MAX_OPTIONAL_BYTES))
    );
}

#[test]
fn final_prepared_bytes_and_query_admission_use_selected_profile() {
    let fixture = Fixture::new();
    fixture
        .authority
        .update(|state| {
            state.capabilities.effective_limits.max_job_input_bytes = 64;
            state.capabilities.binding.descriptor_sha256 = state.capabilities.descriptor_digest();
            Ok(())
        })
        .unwrap();
    fixture.adopt();
    let mut request = preparation();
    request.mandatory_bytes = "火".repeat(30).into_bytes();
    assert!(matches!(fixture.owner.prepare_active(access(), request),
        Err(PolicyOwnerError::Memory(MemoryError::CapabilityLimitExceeded { limit, .. }))
        if limit == "max_job_input_bytes"));
    let mut query = preparation();
    query.query.query = "a".repeat(MAX_QUERY_BYTES + 1);
    assert!(matches!(fixture.owner.prepare_active(access(), query),
        Err(PolicyOwnerError::Memory(MemoryError::CapabilityLimitExceeded { limit, .. }))
        if limit == "max_query_bytes"));
}

#[test]
fn receipts_are_bounded_and_older_receipts_are_not_evicted() {
    let fixture = Fixture::new();
    let raw = bytes(&policy(1, 9000));
    for index in 0..MAX_POLICY_RECEIPTS {
        fixture
            .owner
            .execute(
                access(),
                PolicyCommand::Import {
                    key: format!("request-{index}"),
                    raw: raw.clone(),
                },
            )
            .unwrap();
    }
    assert_eq!(
        fixture.owner.execute(
            access(),
            PolicyCommand::Import {
                key: "over-receipts".to_owned(),
                raw,
            }
        ),
        Err(PolicyOwnerError::Capacity)
    );
    assert_eq!(
        fixture
            .owner
            .lookup_receipt(access(), "request-0")
            .unwrap()
            .sequence,
        1
    );
    assert_eq!(
        fixture.owner.lookup_receipt(access(), "over-receipts"),
        Err(PolicyOwnerError::Missing)
    );
}

#[test]
fn trusted_epochs_reject_non_interoperable_integers_before_publication() {
    let fixture = Fixture::new();
    for field in 0..4 {
        let result = fixture.authority.update(|state| {
            let value = 9_007_199_254_740_992;
            match field {
                0 => state.control_epoch = value,
                1 => state.plan_epoch = value,
                2 => state.owner_epoch = value,
                _ => state.grants.get_mut("policy-grant").unwrap().epoch = value,
            }
            Ok(())
        });
        assert_eq!(
            result,
            Err(if field == 3 {
                PolicyOwnerError::PermissionDenied
            } else {
                PolicyOwnerError::ScopeMismatch
            })
        );
    }
    fixture.adopt();
    fixture
        .owner
        .prepare_active(access(), preparation())
        .unwrap();
}

#[test]
fn recording_consumer_receives_only_admitted_active_selections() {
    fn consume(
        owner: &MemoryPolicyOwner,
        recorded: &mut Vec<String>,
    ) -> Result<(), PolicyOwnerError> {
        let prepared = owner.prepare_active(access(), preparation())?;
        recorded.push(prepared.realized.selection.prepared_manifest_sha256);
        Ok(())
    }
    let fixture = Fixture::new();
    let mut recording = Vec::new();
    assert!(consume(&fixture.owner, &mut recording).is_err());
    assert!(recording.is_empty());
    fixture.adopt();
    consume(&fixture.owner, &mut recording).unwrap();
    assert_eq!(recording.len(), 1);
    fixture
        .authority
        .update(|state| {
            let grant = state.grants.get_mut("policy-grant").unwrap();
            grant.revoked = true;
            grant.epoch += 1;
            Ok(())
        })
        .unwrap();
    assert!(consume(&fixture.owner, &mut recording).is_err());
    assert_eq!(recording.len(), 1);
}
