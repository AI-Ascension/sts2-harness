// SPDX-License-Identifier: MIT

use super::*;
use authority::AuthorizedActor;
use commands::check_review;

impl MemoryPolicyOwner {
    pub(super) fn adopt(
        &self,
        journal: &mut PolicyJournal,
        state: &TrustedPolicyState,
        actor: &AuthorizedActor,
        clock: &dyn PolicyClock,
        review_id: &str,
        digest: &str,
    ) -> Result<String, PolicyOwnerError> {
        let review = journal.review(review_id)?.clone();
        check_review(journal, state, actor, &review, digest)?;
        let approval = journal
            .approvals
            .iter()
            .find(|approval| approval.review_id == review_id)
            .ok_or(PolicyOwnerError::PermissionDenied)?;
        check_approval(state, actor, approval, clock.now_seconds())?;
        if journal
            .adoptions
            .iter()
            .any(|binding| binding.approval_id == approval.approval_id)
        {
            return Err(PolicyOwnerError::Conflict);
        }
        let target = journal.policy(&review.target)?;
        let target_policy = target.typed()?;
        state.validate_target(&target_policy)?;
        if review.kind == ReviewKind::Migration {
            let source = journal.policy(&review.source)?;
            let mut migration = PolicyMigrationProposal::new_from_bytes(
                source.raw_bytes(),
                &state.capabilities,
                &review.review_id,
            )?;
            if migration.violations != review.violations {
                return Err(PolicyOwnerError::StaleReview);
            }
            migration.approve(&approval.approval_id)?;
            let adopted =
                migration.adopt(&target_policy, &state.capabilities, &approval.approval_id)?;
            if adopted != target_policy
                || migration.adopted_policy_sha256.as_deref()
                    != Some(target.execution_sha256.as_str())
            {
                return Err(PolicyOwnerError::Corrupt);
            }
        }
        let version = journal.adoptions.len() as u64 + 1;
        let binding = ActivePolicyBinding {
            binding_id: format!("policy-binding-{version}"),
            version,
            target: review.target,
            target_execution_sha256: review.target_execution_sha256,
            approval_id: approval.approval_id.clone(),
            review_id: review.review_id,
            fence: review.fence,
        };
        let result = binding.binding_id.clone();
        journal.adoptions.push(binding.clone());
        journal.active = Some(binding);
        Ok(result)
    }
}

pub(super) fn check_approval(
    state: &TrustedPolicyState,
    actor: &AuthorizedActor,
    approval: &PolicyApproval,
    now: u64,
) -> Result<(), PolicyOwnerError> {
    if approval.subject != actor.subject
        || approval.grant_id != actor.grant_id
        || approval.grant_epoch != actor.grant_epoch
        || approval.expires_at <= now
        || approval.approved_at > now
    {
        return Err(PolicyOwnerError::PermissionDenied);
    }
    state.check_actor(actor, PolicyPermission::Approve, now)
}
