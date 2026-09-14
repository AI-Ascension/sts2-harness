// SPDX-License-Identifier: MIT

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use sts2_harness::context_memory::{policy_owner::*, *};
use sts2_harness::management::{AuthContext, StaticAuthenticator};

pub struct Clock(pub AtomicU64);
impl PolicyClock for Clock {
    fn now_seconds(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
    fn now_timestamp(&self) -> String {
        "2026-09-14T12:00:00Z".to_owned()
    }
}

pub fn scope() -> MemoryScope {
    MemoryScope::new("project", "run", "episode", "agent")
}
pub fn access() -> PolicyAccess<'static> {
    PolicyAccess {
        bearer: Some("synthetic-owner-token"),
        grant_id: "policy-grant",
    }
}
pub fn policy(version: u64, budget: usize) -> MemoryPolicy {
    MemoryPolicy {
        schema: MEMORY_POLICY_SCHEMA.to_owned(),
        policy_id: "saved-policy".to_owned(),
        version,
        scope: scope(),
        mode: PolicyMode::ManualSnapshot,
        status: PolicyStatus::Approved,
        phase2_revision_id: Some("revision-1".to_owned()),
        corpus_generation: 1,
        rolling_same_episode_sources: false,
        cross_scope: false,
        approved_summary_catalog: Vec::new(),
        ranker_version: "lexical-v1".to_owned(),
        query_derivation_version: "derive-v1".to_owned(),
        max_candidates: 64,
        max_results: 16,
        max_selected: 32,
        optional_byte_budget: budget,
        fallback: SelectionFallback::Block,
        automatic_summary_activation: false,
        generate_during_selection: false,
        authorization_policy_version: "auth-v1".to_owned(),
    }
}
pub fn bytes(policy: &MemoryPolicy) -> Vec<u8> {
    let encoded = serde_json::to_string_pretty(policy).unwrap();
    // A noncanonical, schema-valid encoding whose raw hash differs from typed serialization.
    format!(
        "\n{}\n ",
        encoded.replace("saved-policy", r"saved-\u0070olicy")
    )
    .into_bytes()
}
pub fn reference(raw: &[u8]) -> SavedPolicyRef {
    let policy: MemoryPolicy = serde_json::from_slice(raw).unwrap();
    SavedPolicyRef {
        policy_id: policy.policy_id,
        version: policy.version,
        raw_sha256: sha256_hex(raw),
    }
}
pub fn entry(id: &str, generation: u64) -> MemoryEntry {
    MemoryEntry::new(
        scope(),
        id,
        format!("record-{id}"),
        MemoryKind::HistoricalObservation,
        EvidenceStatus::Observed,
        "branch",
        id,
        b"synthetic settled observation".to_vec(),
        1,
        2,
        generation,
        "2026-09-14T10:00:00Z",
        "2030-09-14T10:00:00Z",
        "synthetic-v1",
        false,
    )
}
pub fn state() -> TrustedPolicyState {
    let mut corpus = MemoryCorpus::with_limits(scope(), 16, 4096).unwrap();
    corpus.admit(entry("history", 1)).unwrap();
    let capabilities = corpus.capabilities();
    let grant = PolicyGrant {
        grant_id: "policy-grant".to_owned(),
        subject: "operator".to_owned(),
        scope: scope(),
        permissions: BTreeSet::from([
            PolicyPermission::ReadMetadata,
            PolicyPermission::ReadContent,
            PolicyPermission::Write,
            PolicyPermission::Approve,
            PolicyPermission::Adopt,
            PolicyPermission::Select,
        ]),
        epoch: 1,
        expires_at: 1000,
        revoked: false,
    };
    TrustedPolicyState {
        trusted_owner_revision: capabilities.binding.owner_revision.clone(),
        trusted_adapter_revision: capabilities.binding.adapter_revision.clone(),
        trusted_policy_schema_sha256: memory_policy_schema_sha256(),
        corpus,
        capabilities,
        phase2_revision_id: "revision-1".to_owned(),
        control_epoch: 1,
        plan_epoch: 1,
        owner_epoch: 1,
        grants: BTreeMap::from([(grant.grant_id.clone(), grant)]),
    }
}
pub fn authenticator() -> Arc<StaticAuthenticator> {
    Arc::new(
        StaticAuthenticator::single(
            "synthetic-owner-token",
            AuthContext::new("operator", ["workflow:*".to_owned()]).unwrap(),
        )
        .unwrap()
        .with_credential(
            "other-token",
            AuthContext::new("other", ["workflow:*".to_owned()]).unwrap(),
        )
        .unwrap(),
    )
}
pub fn authority(clock: Arc<dyn PolicyClock>) -> Arc<MemoryPolicyAuthority> {
    Arc::new(MemoryPolicyAuthority::new(state(), authenticator(), clock).unwrap())
}
pub struct Fixture {
    pub directory: PathBuf,
    pub path: PathBuf,
    pub authority: Arc<MemoryPolicyAuthority>,
    pub clock: Arc<Clock>,
    pub owner: MemoryPolicyOwner,
    cleanup: Cleanup,
}
impl Fixture {
    pub fn new() -> Self {
        let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/policy-owner-tests")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("policies.sqlite");
        let clock = Arc::new(Clock(AtomicU64::new(100)));
        let authority = authority(clock.clone());
        let owner = MemoryPolicyOwner::open(
            &path,
            [7; 32],
            authority.clone(),
            PolicyStoreConsent::SyntheticOnly,
        )
        .unwrap();
        Self {
            cleanup: Cleanup(directory.clone()),
            directory,
            path,
            authority,
            clock,
            owner,
        }
    }
    pub fn import(&self) -> Vec<u8> {
        let raw = bytes(&policy(1, MEMORY_POLICY_SCHEMA_MAX_OPTIONAL_BYTES));
        self.owner
            .execute(
                access(),
                PolicyCommand::Import {
                    key: "import".to_owned(),
                    raw: raw.clone(),
                },
            )
            .unwrap();
        raw
    }
    pub fn propose(&self) -> PolicyReview {
        let raw = self.import();
        self.owner
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
        self.owner.inspect_review(access(), "review").unwrap()
    }
    pub fn approve(&self, review: &PolicyReview) {
        self.owner
            .execute(
                access(),
                PolicyCommand::Approve {
                    key: format!("approve-{}", review.review_id),
                    review_id: review.review_id.clone(),
                    review_sha256: review.review_sha256.clone(),
                },
            )
            .unwrap();
    }
    pub fn adopt_command(review: &PolicyReview) -> PolicyCommand {
        PolicyCommand::Adopt {
            key: format!("adopt-{}", review.review_id),
            review_id: review.review_id.clone(),
            review_sha256: review.review_sha256.clone(),
        }
    }
    pub fn adopt(&self) -> PolicyReview {
        let review = self.propose();
        self.approve(&review);
        self.owner
            .execute(access(), Self::adopt_command(&review))
            .unwrap();
        review
    }
}
struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
pub fn preparation() -> ActivePolicyPreparation {
    ActivePolicyPreparation {
        selection_id: "selection".to_owned(),
        query: MemoryQuery {
            schema: MEMORY_QUERY_SCHEMA.to_owned(),
            query_id: "query".to_owned(),
            scope: scope(),
            branch_id: "branch".to_owned(),
            query: "settled".to_owned(),
            cutoff: 10,
            corpus_generation: 1,
            ranker_version: "lexical-v1".to_owned(),
            limit: 16,
            max_candidates: 64,
            effect_class: "local_read_no_inference".to_owned(),
        },
        mandatory_bytes: "mandatory legality: 火".as_bytes().to_vec(),
        phase2_prepared_manifest_sha256: sha256_hex(b"phase2"),
        prepared_content_ref: "prepared".to_owned(),
        expires_at: "2026-09-15T12:00:00Z".to_owned(),
    }
}
