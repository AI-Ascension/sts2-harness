// SPDX-License-Identifier: MIT

use super::*;
use authority::AuthorizedActor;

impl MemoryPolicyOwner {
    pub(super) fn apply(
        &self,
        journal: &mut PolicyJournal,
        state: &TrustedPolicyState,
        actor: &AuthorizedActor,
        clock: &dyn PolicyClock,
        command: &PolicyCommand,
    ) -> Result<String, PolicyOwnerError> {
        match command {
            PolicyCommand::Import { raw, .. } => {
                let policy = SavedPolicy::new(raw, &journal.scope)?;
                let result = format!(
                    "{}:{}",
                    policy.reference.policy_id, policy.reference.version
                );
                journal.insert_policy(policy)?;
                Ok(result)
            }
            PolicyCommand::ProposeMigration {
                review_id,
                source,
                target_raw,
                expected_active_version,
                ..
            } => self.propose(
                journal,
                state,
                actor,
                review_id,
                source,
                target_raw,
                *expected_active_version,
                ReviewKind::Migration,
            ),
            PolicyCommand::ProposeRevalidation {
                review_id,
                source,
                target_raw,
                expected_active_version,
                ..
            } => self.propose(
                journal,
                state,
                actor,
                review_id,
                source,
                target_raw,
                Some(*expected_active_version),
                ReviewKind::Revalidation,
            ),
            PolicyCommand::Approve {
                review_id,
                review_sha256,
                ..
            } => self.approve(journal, state, actor, clock, review_id, review_sha256),
            PolicyCommand::Adopt {
                review_id,
                review_sha256,
                ..
            } => self.adopt(journal, state, actor, clock, review_id, review_sha256),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn propose(
        &self,
        journal: &mut PolicyJournal,
        state: &TrustedPolicyState,
        actor: &AuthorizedActor,
        review_id: &str,
        source_ref: &SavedPolicyRef,
        target_raw: &[u8],
        expected_active_version: Option<u64>,
        kind: ReviewKind,
    ) -> Result<String, PolicyOwnerError> {
        if journal.reviews.len() >= MAX_POLICY_REVIEWS {
            return Err(PolicyOwnerError::Capacity);
        }
        if journal
            .reviews
            .iter()
            .any(|review| review.review_id == review_id)
        {
            return Err(PolicyOwnerError::Conflict);
        }
        let current_active = journal.active.as_ref().map(|active| active.version);
        if expected_active_version != current_active {
            return Err(PolicyOwnerError::StaleReview);
        }
        let source = journal.policy(source_ref)?;
        let target = SavedPolicy::new(target_raw, &journal.scope)?;
        if target.reference.policy_id != source.reference.policy_id {
            return Err(PolicyOwnerError::ScopeMismatch);
        }
        let violations = validate_lineage(journal, state, source, &target, review_id, kind)?;
        state.validate_target(&target.typed()?)?;
        let mut review = PolicyReview {
            review_id: review_id.to_owned(),
            kind,
            source: source_ref.clone(),
            target: target.reference.clone(),
            target_execution_sha256: target.execution_sha256.clone(),
            fence: PolicyFence::current(state, actor, journal),
            violations,
            review_sha256: String::new(),
        };
        review.review_sha256 = review.digest()?;
        journal.insert_policy(target)?;
        journal.reviews.push(review);
        Ok(review_id.to_owned())
    }

    fn approve(
        &self,
        journal: &mut PolicyJournal,
        state: &TrustedPolicyState,
        actor: &AuthorizedActor,
        clock: &dyn PolicyClock,
        review_id: &str,
        digest: &str,
    ) -> Result<String, PolicyOwnerError> {
        let review = journal.review(review_id)?;
        check_review(journal, state, actor, review, digest)?;
        if journal
            .approvals
            .iter()
            .any(|approval| approval.review_id == review_id)
        {
            return Err(PolicyOwnerError::Conflict);
        }
        let grant = state
            .grants
            .get(&actor.grant_id)
            .ok_or(PolicyOwnerError::PermissionDenied)?;
        let approval_id = format!("policy-approval-{}", journal.approvals.len() + 1);
        journal.approvals.push(PolicyApproval {
            approval_id: approval_id.clone(),
            review_id: review_id.to_owned(),
            review_sha256: digest.to_owned(),
            subject: actor.subject.clone(),
            grant_id: actor.grant_id.clone(),
            grant_epoch: actor.grant_epoch,
            approved_at: clock.now_seconds(),
            expires_at: grant.expires_at,
        });
        Ok(approval_id)
    }
}

fn validate_lineage(
    journal: &PolicyJournal,
    state: &TrustedPolicyState,
    source: &SavedPolicy,
    target: &SavedPolicy,
    review_id: &str,
    kind: ReviewKind,
) -> Result<Vec<MemoryLimitViolation>, PolicyOwnerError> {
    if let Some(active) = &journal.active
        && (active.target.policy_id != target.reference.policy_id
            || target.reference.version < active.target.version
            || (kind == ReviewKind::Migration && target.reference.version == active.target.version))
    {
        return Err(PolicyOwnerError::Conflict);
    }
    match kind {
        ReviewKind::Migration => {
            if target.reference.version <= source.reference.version {
                return Err(PolicyOwnerError::Conflict);
            }
            Ok(PolicyMigrationProposal::new_from_bytes(
                source.raw_bytes(),
                &state.capabilities,
                review_id,
            )?
            .violations)
        }
        ReviewKind::Revalidation => {
            let active = journal.active.as_ref().ok_or(PolicyOwnerError::Missing)?;
            if active.target != source.reference
                || !journal.adoptions.iter().any(|old| old == active)
            {
                return Err(PolicyOwnerError::StaleReview);
            }
            if target.reference.version < source.reference.version
                || (target.reference.version == source.reference.version && target != source)
            {
                return Err(PolicyOwnerError::Conflict);
            }
            Ok(Vec::new())
        }
    }
}

pub(super) fn check_review(
    journal: &PolicyJournal,
    state: &TrustedPolicyState,
    actor: &AuthorizedActor,
    review: &PolicyReview,
    digest: &str,
) -> Result<(), PolicyOwnerError> {
    if review.review_sha256 != digest
        || review.digest()? != digest
        || review.fence != PolicyFence::current(state, actor, journal)
    {
        return Err(PolicyOwnerError::StaleReview);
    }
    state.validate_target(&journal.policy(&review.target)?.typed()?)?;
    Ok(())
}
