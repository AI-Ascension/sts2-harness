// SPDX-License-Identifier: MIT

use super::{TrustedPolicyState, authority::AuthorizedActor, original::parse_original, types::*};
use crate::context_memory::*;
use serde::{Deserialize, Serialize};

#[cfg(test)]
#[path = "policy_owner_decode_tests.rs"]
mod decode_tests;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedPolicyRef {
    pub policy_id: String,
    pub version: u64,
    pub raw_sha256: String,
}
impl SavedPolicyRef {
    pub(super) fn valid(&self) -> bool {
        valid_id(&self.policy_id)
            && self.version > 0
            && self.version <= 9_007_199_254_740_991
            && valid_digest(&self.raw_sha256)
    }
}

/// Exact bytes are disclosed only through the content-authorized inspection operation.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedPolicy {
    pub reference: SavedPolicyRef,
    pub execution_sha256: String,
    #[serde(deserialize_with = "bounded_raw")]
    raw: Vec<u8>,
}
impl SavedPolicy {
    pub fn raw_bytes(&self) -> &[u8] {
        &self.raw
    }
    pub(super) fn new(raw: &[u8], scope: &MemoryScope) -> Result<Self, PolicyOwnerError> {
        let policy = parse_original(raw, scope)?;
        let value = Self {
            reference: SavedPolicyRef {
                policy_id: policy.policy_id.clone(),
                version: policy.version,
                raw_sha256: sha256_hex(raw),
            },
            execution_sha256: sha256_hex(
                serde_json::to_vec(&policy).map_err(|_| PolicyOwnerError::SchemaInvalid)?,
            ),
            raw: raw.to_vec(),
        };
        if !value.reference.valid() {
            return Err(PolicyOwnerError::SchemaInvalid);
        }
        Ok(value)
    }
    pub(super) fn typed(&self) -> Result<MemoryPolicy, PolicyOwnerError> {
        parse_strict_json(&self.raw).map_err(|_| PolicyOwnerError::Corrupt)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyFence {
    pub store_epoch: u64,
    pub owner_epoch: u64,
    pub descriptor_sha256: String,
    pub phase2_revision_id: String,
    pub control_epoch: u64,
    pub plan_epoch: u64,
    pub corpus_generation: u64,
    pub revocation_epoch: u64,
    pub grant_id: String,
    pub grant_epoch: u64,
    pub expected_active_version: Option<u64>,
}
impl PolicyFence {
    pub(super) fn valid(&self) -> bool {
        [
            self.store_epoch,
            self.owner_epoch,
            self.control_epoch,
            self.plan_epoch,
            self.corpus_generation,
            self.grant_epoch,
        ]
        .iter()
        .all(|value| *value > 0 && *value <= 9_007_199_254_740_991)
            && self.revocation_epoch <= 9_007_199_254_740_991
            && valid_id(&self.phase2_revision_id)
            && valid_id(&self.grant_id)
            && valid_digest(&self.descriptor_sha256)
            && self
                .expected_active_version
                .is_none_or(|value| value > 0 && value <= MAX_POLICY_REVIEWS as u64)
    }
    pub(super) fn current(
        state: &TrustedPolicyState,
        actor: &AuthorizedActor,
        journal: &PolicyJournal,
    ) -> Self {
        Self {
            store_epoch: journal.store_epoch,
            owner_epoch: state.owner_epoch,
            descriptor_sha256: state.capabilities.binding.descriptor_sha256.clone(),
            phase2_revision_id: state.phase2_revision_id.clone(),
            control_epoch: state.control_epoch,
            plan_epoch: state.plan_epoch,
            corpus_generation: state.corpus.generation(),
            revocation_epoch: state.corpus.revocation_epoch(),
            grant_id: actor.grant_id.clone(),
            grant_epoch: actor.grant_epoch,
            expected_active_version: journal.active.as_ref().map(|active| active.version),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewKind {
    Migration,
    Revalidation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyReview {
    pub review_id: String,
    pub kind: ReviewKind,
    pub source: SavedPolicyRef,
    pub target: SavedPolicyRef,
    pub target_execution_sha256: String,
    pub fence: PolicyFence,
    pub violations: Vec<MemoryLimitViolation>,
    pub review_sha256: String,
}
impl PolicyReview {
    pub(super) fn digest(&self) -> Result<String, PolicyOwnerError> {
        let mut value = self.clone();
        value.review_sha256.clear();
        Ok(sha256_hex(
            serde_json::to_vec(&value).map_err(|_| PolicyOwnerError::Corrupt)?,
        ))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyApproval {
    pub approval_id: String,
    pub review_id: String,
    pub review_sha256: String,
    pub subject: String,
    pub grant_id: String,
    pub grant_epoch: u64,
    pub approved_at: u64,
    pub expires_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivePolicyBinding {
    pub binding_id: String,
    pub version: u64,
    pub target: SavedPolicyRef,
    pub target_execution_sha256: String,
    pub approval_id: String,
    pub review_id: String,
    pub fence: PolicyFence,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyReceipt {
    pub operation_id: String,
    pub idempotency_key: String,
    pub request_sha256: String,
    pub subject: String,
    pub operation: String,
    pub result_id: String,
    pub sequence: u64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PolicyJournal {
    pub schema: String,
    pub scope: MemoryScope,
    pub store_epoch: u64,
    #[serde(deserialize_with = "bounded_policies")]
    pub policies: Vec<SavedPolicy>,
    #[serde(deserialize_with = "bounded_reviews")]
    pub reviews: Vec<PolicyReview>,
    #[serde(deserialize_with = "bounded_approvals")]
    pub approvals: Vec<PolicyApproval>,
    #[serde(deserialize_with = "bounded_adoptions")]
    pub adoptions: Vec<ActivePolicyBinding>,
    pub active: Option<ActivePolicyBinding>,
    #[serde(deserialize_with = "bounded_receipts")]
    pub receipts: Vec<PolicyReceipt>,
}

fn bounded_vec<'de, D, T, const MAX: usize>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Bounded<T, const MAX: usize>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>, const MAX: usize> serde::de::Visitor<'de> for Bounded<T, MAX> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "at most {MAX} policy records")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<T>, A::Error> {
            let mut values = Vec::new();
            while values.len() < MAX {
                let Some(value) = seq.next_element()? else {
                    return Ok(values);
                };
                values.push(value);
            }
            if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                return Err(serde::de::Error::custom("policy record capacity"));
            }
            Ok(values)
        }
    }
    deserializer.deserialize_seq(Bounded::<T, MAX>(std::marker::PhantomData))
}
fn bounded_policies<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<SavedPolicy>, D::Error> {
    bounded_vec::<D, SavedPolicy, MAX_POLICY_VERSIONS>(d)
}
fn bounded_raw<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
    bounded_vec::<D, u8, MAX_POLICY_BYTES>(d)
}
fn bounded_reviews<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<PolicyReview>, D::Error> {
    bounded_vec::<D, PolicyReview, MAX_POLICY_REVIEWS>(d)
}
fn bounded_approvals<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Vec<PolicyApproval>, D::Error> {
    bounded_vec::<D, PolicyApproval, MAX_POLICY_REVIEWS>(d)
}
fn bounded_adoptions<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Vec<ActivePolicyBinding>, D::Error> {
    bounded_vec::<D, ActivePolicyBinding, MAX_POLICY_REVIEWS>(d)
}
fn bounded_receipts<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Vec<PolicyReceipt>, D::Error> {
    bounded_vec::<D, PolicyReceipt, MAX_POLICY_RECEIPTS>(d)
}

impl PolicyJournal {
    pub fn empty(scope: MemoryScope) -> Self {
        Self {
            schema: "ascension.context-memory.policy-store.v1".to_owned(),
            scope,
            store_epoch: 1,
            policies: Vec::new(),
            reviews: Vec::new(),
            approvals: Vec::new(),
            adoptions: Vec::new(),
            active: None,
            receipts: Vec::new(),
        }
    }
    pub fn policy(&self, reference: &SavedPolicyRef) -> Result<&SavedPolicy, PolicyOwnerError> {
        self.policies
            .iter()
            .find(|policy| &policy.reference == reference)
            .ok_or(PolicyOwnerError::Missing)
    }
    pub fn review(&self, id: &str) -> Result<&PolicyReview, PolicyOwnerError> {
        self.reviews
            .iter()
            .find(|review| review.review_id == id)
            .ok_or(PolicyOwnerError::Missing)
    }
    pub fn insert_policy(&mut self, policy: SavedPolicy) -> Result<(), PolicyOwnerError> {
        if let Some(old) = self.policies.iter().find(|old| {
            old.reference.policy_id == policy.reference.policy_id
                && old.reference.version == policy.reference.version
        }) {
            return if old == &policy {
                Ok(())
            } else {
                Err(PolicyOwnerError::Conflict)
            };
        }
        if self.policies.len() >= MAX_POLICY_VERSIONS {
            return Err(PolicyOwnerError::Capacity);
        }
        self.policies.push(policy);
        Ok(())
    }
}
