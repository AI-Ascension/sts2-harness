// SPDX-License-Identifier: MIT

use super::{records::*, types::*};
use std::collections::BTreeSet;

impl PolicyJournal {
    pub fn validate(&self) -> Result<(), PolicyOwnerError> {
        if self.schema != "ascension.context-memory.policy-store.v1"
            || self.store_epoch == 0
            || self.policies.len() > MAX_POLICY_VERSIONS
            || self.reviews.len() > MAX_POLICY_REVIEWS
            || self.approvals.len() > MAX_POLICY_REVIEWS
            || self.adoptions.len() > MAX_POLICY_REVIEWS
            || self.receipts.len() > MAX_POLICY_RECEIPTS
        {
            return Err(PolicyOwnerError::Capacity);
        }
        self.validate_policies()?;
        self.validate_reviews()?;
        self.validate_approvals()?;
        self.validate_adoptions()?;
        self.validate_receipts()
    }

    fn validate_policies(&self) -> Result<(), PolicyOwnerError> {
        let mut policy_keys = BTreeSet::new();
        for policy in &self.policies {
            let valid = SavedPolicy::new(policy.raw_bytes(), &self.scope)?;
            if &valid != policy
                || !policy_keys.insert((&policy.reference.policy_id, policy.reference.version))
            {
                return Err(PolicyOwnerError::Corrupt);
            }
        }
        Ok(())
    }

    fn validate_reviews(&self) -> Result<(), PolicyOwnerError> {
        let mut review_ids = BTreeSet::new();
        for review in &self.reviews {
            if !valid_id(&review.review_id)
                || !review_ids.insert(&review.review_id)
                || review.digest()? != review.review_sha256
                || review.violations.len() > 4
                || (review.kind == ReviewKind::Migration && review.violations.is_empty())
                || (review.kind == ReviewKind::Revalidation && !review.violations.is_empty())
                || review.source.policy_id != review.target.policy_id
                || review.target.version < review.source.version
                || (review.kind == ReviewKind::Migration
                    && review.target.version == review.source.version)
                || !review.fence.valid()
                || review.target_execution_sha256 != self.policy(&review.target)?.execution_sha256
            {
                return Err(PolicyOwnerError::Corrupt);
            }
            self.policy(&review.source)?;
        }
        Ok(())
    }

    fn validate_approvals(&self) -> Result<(), PolicyOwnerError> {
        let mut approvals = BTreeSet::new();
        let mut approved_reviews = BTreeSet::new();
        for (index, approval) in self.approvals.iter().enumerate() {
            if approval.approval_id != format!("policy-approval-{}", index + 1)
                || !valid_id(&approval.subject)
                || !approvals.insert(&approval.approval_id)
                || !approved_reviews.insert(&approval.review_id)
                || approval.review_sha256 != self.review(&approval.review_id)?.review_sha256
                || approval.approved_at >= approval.expires_at
                || approval.grant_id != self.review(&approval.review_id)?.fence.grant_id
                || approval.grant_epoch != self.review(&approval.review_id)?.fence.grant_epoch
            {
                return Err(PolicyOwnerError::Corrupt);
            }
        }
        Ok(())
    }

    fn validate_adoptions(&self) -> Result<(), PolicyOwnerError> {
        let mut used_approvals = BTreeSet::new();
        let mut prior: Option<&ActivePolicyBinding> = None;
        for (index, binding) in self.adoptions.iter().enumerate() {
            let review = self.review(&binding.review_id)?;
            if binding.version != index as u64 + 1
                || binding.binding_id != format!("policy-binding-{}", binding.version)
                || binding.fence != review.fence
                || binding.target != review.target
                || binding.target_execution_sha256 != self.policy(&binding.target)?.execution_sha256
                || !used_approvals.insert(&binding.approval_id)
                || !self.approvals.iter().any(|approval| {
                    approval.approval_id == binding.approval_id
                        && approval.review_id == binding.review_id
                })
                || binding.fence.expected_active_version != prior.map(|value| value.version)
            {
                return Err(PolicyOwnerError::Corrupt);
            }
            if let Some(previous) = prior
                && (binding.target.policy_id != previous.target.policy_id
                    || binding.target.version < previous.target.version
                    || (binding.target.version == previous.target.version
                        && binding.target != previous.target))
            {
                return Err(PolicyOwnerError::Corrupt);
            }
            prior = Some(binding);
        }
        if self.active.as_ref() != self.adoptions.last() {
            return Err(PolicyOwnerError::Corrupt);
        }
        Ok(())
    }
}
