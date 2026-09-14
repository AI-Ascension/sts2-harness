// SPDX-License-Identifier: MIT

use super::{records::*, types::*};
use std::collections::BTreeSet;

impl PolicyJournal {
    pub(super) fn validate_receipts(&self) -> Result<(), PolicyOwnerError> {
        let mut keys = BTreeSet::new();
        let mut history = ReceiptHistory::default();
        for (index, receipt) in self.receipts.iter().enumerate() {
            if receipt.sequence != index as u64 + 1
                || receipt.operation_id != format!("policy-operation-{}", receipt.sequence)
                || !valid_id(&receipt.idempotency_key)
                || !valid_id(&receipt.subject)
                || !valid_digest(&receipt.request_sha256)
                || receipt.result_id.len() > 256
                || !keys.insert((&receipt.subject, &receipt.idempotency_key))
            {
                return Err(PolicyOwnerError::Corrupt);
            }
            // Only reconstruct the stored command payload for hashing. Never execute it or
            // infer historical credentials/current grant validity from these records.
            let command = history.command(self, receipt)?;
            command.validate().map_err(|_| PolicyOwnerError::Corrupt)?;
            if command
                .fingerprint()
                .map_err(|_| PolicyOwnerError::Corrupt)?
                != receipt.request_sha256
            {
                return Err(PolicyOwnerError::Corrupt);
            }
        }
        if history.policies != self.policies.len()
            || history.reviews != self.reviews.len()
            || history.approvals != self.approvals.len()
            || history.adoptions != self.adoptions.len()
        {
            return Err(PolicyOwnerError::Corrupt);
        }
        Ok(())
    }
}

/// Counts refer to immutable prefixes of the already bounded, validated record vectors.
#[derive(Default)]
struct ReceiptHistory {
    policies: usize,
    reviews: usize,
    approvals: usize,
    adoptions: usize,
}

impl ReceiptHistory {
    fn command(
        &mut self,
        journal: &PolicyJournal,
        receipt: &PolicyReceipt,
    ) -> Result<PolicyCommand, PolicyOwnerError> {
        match receipt.operation.as_str() {
            "import" => {
                let index = journal
                    .policies
                    .iter()
                    .position(|policy| {
                        receipt.result_id
                            == format!(
                                "{}:{}",
                                policy.reference.policy_id, policy.reference.version
                            )
                    })
                    .ok_or(PolicyOwnerError::Corrupt)?;
                self.introduce_policy(index)?;
                Ok(PolicyCommand::Import {
                    key: receipt.idempotency_key.clone(),
                    raw: journal.policies[index].raw_bytes().to_vec(),
                })
            }
            "propose_migration" => self.proposal(journal, receipt, ReviewKind::Migration),
            "propose_revalidation" => self.proposal(journal, receipt, ReviewKind::Revalidation),
            "approve" => self.approval(journal, receipt),
            "adopt" => self.adoption(journal, receipt),
            _ => Err(PolicyOwnerError::Corrupt),
        }
    }

    fn introduce_policy(&mut self, index: usize) -> Result<(), PolicyOwnerError> {
        if index > self.policies {
            return Err(PolicyOwnerError::Corrupt);
        }
        if index == self.policies {
            self.policies += 1;
        }
        Ok(())
    }

    fn proposal(
        &mut self,
        journal: &PolicyJournal,
        receipt: &PolicyReceipt,
        kind: ReviewKind,
    ) -> Result<PolicyCommand, PolicyOwnerError> {
        let review = journal
            .reviews
            .get(self.reviews)
            .ok_or(PolicyOwnerError::Corrupt)?;
        if review.review_id != receipt.result_id
            || review.kind != kind
            || policy_index(journal, &review.source)? >= self.policies
        {
            return Err(PolicyOwnerError::Corrupt);
        }
        self.check_prior_adoption(journal, review)?;
        let target = policy_index(journal, &review.target)?;
        self.introduce_policy(target)?;
        self.reviews += 1;
        let key = receipt.idempotency_key.clone();
        let review_id = review.review_id.clone();
        let source = review.source.clone();
        let target_raw = journal.policies[target].raw_bytes().to_vec();
        let expected_active_version = review.fence.expected_active_version;
        Ok(match kind {
            ReviewKind::Migration => PolicyCommand::ProposeMigration {
                key,
                review_id,
                source,
                target_raw,
                expected_active_version,
            },
            ReviewKind::Revalidation => PolicyCommand::ProposeRevalidation {
                key,
                review_id,
                source,
                target_raw,
                expected_active_version: expected_active_version
                    .ok_or(PolicyOwnerError::Corrupt)?,
            },
        })
    }

    fn check_prior_adoption(
        &self,
        journal: &PolicyJournal,
        review: &PolicyReview,
    ) -> Result<(), PolicyOwnerError> {
        let active = self
            .adoptions
            .checked_sub(1)
            .and_then(|index| journal.adoptions.get(index));
        if review.fence.expected_active_version != active.map(|binding| binding.version) {
            return Err(PolicyOwnerError::Corrupt);
        }
        if let Some(active) = active
            && (review.target.policy_id != active.target.policy_id
                || review.target.version < active.target.version
                || (review.kind == ReviewKind::Migration
                    && review.target.version == active.target.version)
                || (review.kind == ReviewKind::Revalidation && review.source != active.target))
        {
            return Err(PolicyOwnerError::Corrupt);
        }
        if active.is_none() && review.kind == ReviewKind::Revalidation {
            return Err(PolicyOwnerError::Corrupt);
        }
        Ok(())
    }

    fn approval(
        &mut self,
        journal: &PolicyJournal,
        receipt: &PolicyReceipt,
    ) -> Result<PolicyCommand, PolicyOwnerError> {
        let approval = journal
            .approvals
            .get(self.approvals)
            .ok_or(PolicyOwnerError::Corrupt)?;
        if approval.approval_id != receipt.result_id
            || approval.subject != receipt.subject
            || review_index(journal, &approval.review_id)? >= self.reviews
            || proposal_subject(journal, &approval.review_id, receipt.sequence)? != approval.subject
        {
            return Err(PolicyOwnerError::Corrupt);
        }
        self.approvals += 1;
        Ok(PolicyCommand::Approve {
            key: receipt.idempotency_key.clone(),
            review_id: approval.review_id.clone(),
            review_sha256: approval.review_sha256.clone(),
        })
    }

    fn adoption(
        &mut self,
        journal: &PolicyJournal,
        receipt: &PolicyReceipt,
    ) -> Result<PolicyCommand, PolicyOwnerError> {
        let binding = journal
            .adoptions
            .get(self.adoptions)
            .ok_or(PolicyOwnerError::Corrupt)?;
        let approval_index = journal
            .approvals
            .iter()
            .position(|approval| approval.approval_id == binding.approval_id)
            .ok_or(PolicyOwnerError::Corrupt)?;
        if binding.binding_id != receipt.result_id
            || approval_index >= self.approvals
            || journal.approvals[approval_index].subject != receipt.subject
            || review_index(journal, &binding.review_id)? >= self.reviews
        {
            return Err(PolicyOwnerError::Corrupt);
        }
        self.adoptions += 1;
        let review = journal
            .review(&binding.review_id)
            .map_err(|_| PolicyOwnerError::Corrupt)?;
        Ok(PolicyCommand::Adopt {
            key: receipt.idempotency_key.clone(),
            review_id: binding.review_id.clone(),
            review_sha256: review.review_sha256.clone(),
        })
    }
}

fn policy_index(
    journal: &PolicyJournal,
    reference: &SavedPolicyRef,
) -> Result<usize, PolicyOwnerError> {
    journal
        .policies
        .iter()
        .position(|policy| policy.reference == *reference)
        .ok_or(PolicyOwnerError::Corrupt)
}

fn review_index(journal: &PolicyJournal, review_id: &str) -> Result<usize, PolicyOwnerError> {
    journal
        .reviews
        .iter()
        .position(|review| review.review_id == review_id)
        .ok_or(PolicyOwnerError::Corrupt)
}

fn proposal_subject<'a>(
    journal: &'a PolicyJournal,
    review_id: &str,
    before: u64,
) -> Result<&'a str, PolicyOwnerError> {
    journal
        .receipts
        .iter()
        .take_while(|receipt| receipt.sequence < before)
        .find(|receipt| {
            receipt.result_id == review_id
                && matches!(
                    receipt.operation.as_str(),
                    "propose_migration" | "propose_revalidation"
                )
        })
        .map(|receipt| receipt.subject.as_str())
        .ok_or(PolicyOwnerError::Corrupt)
}
