// SPDX-License-Identifier: MIT

use super::*;
use authority::AuthorizedActor;

/// Local preparation inputs. There is intentionally no policy field.
pub struct ActivePolicyPreparation {
    pub selection_id: String,
    pub query: MemoryQuery,
    pub mandatory_bytes: Vec<u8>,
    pub phase2_prepared_manifest_sha256: String,
    pub prepared_content_ref: String,
    pub expires_at: String,
}

pub struct PreparedActivePolicy {
    pub binding: ActivePolicyBinding,
    pub realized: RealizedSelection,
}

impl MemoryPolicyOwner {
    /// Loads the actual durable active target and realizes it against the leased owner corpus.
    /// The caller dispatching a provider must separately admit the final request; this is not a
    /// gameplay/provider operation and does not perform any callback under the authority lock.
    pub fn prepare_active(
        &self,
        access: PolicyAccess<'_>,
        request: ActivePolicyPreparation,
    ) -> Result<PreparedActivePolicy, PolicyOwnerError> {
        if request.mandatory_bytes.len() > MAX_JOB_INPUT_BYTES {
            return Err(PolicyOwnerError::Capacity);
        }
        if !types::valid_id(&request.selection_id)
            || !types::valid_id(&request.query.query_id)
            || !types::valid_id(&request.prepared_content_ref)
            || !types::valid_digest(&request.phase2_prepared_manifest_sha256)
            || request.expires_at.len() != 20
        {
            return Err(PolicyOwnerError::SchemaInvalid);
        }
        self.authority
            .with_authorized(access, PolicyPermission::Select, |state, actor, clock| {
                let mut store = self
                    .store
                    .lock()
                    .map_err(|_| PolicyOwnerError::Unavailable)?;
                store.read_lease(|journal| {
                    let active = admitted_active(journal, state, actor, clock.now_seconds())?;
                    let realized =
                        realize_active(state, &active.policy, request, &clock.now_timestamp())?;
                    check_prepare_grants(
                        state,
                        actor,
                        &active.approver,
                        &active.approval,
                        clock.now_seconds(),
                    )?;
                    Ok(PreparedActivePolicy {
                        binding: active.binding,
                        realized,
                    })
                })
            })
    }
}

struct AdmittedPolicy {
    binding: ActivePolicyBinding,
    approval: PolicyApproval,
    approver: AuthorizedActor,
    policy: MemoryPolicy,
}

fn admitted_active(
    journal: &PolicyJournal,
    state: &TrustedPolicyState,
    actor: &AuthorizedActor,
    now: u64,
) -> Result<AdmittedPolicy, PolicyOwnerError> {
    let binding = journal.active.as_ref().ok_or(PolicyOwnerError::Missing)?;
    let review = journal.review(&binding.review_id)?;
    let approval = journal
        .approvals
        .iter()
        .find(|approval| approval.approval_id == binding.approval_id)
        .ok_or(PolicyOwnerError::Corrupt)?;
    let approver = AuthorizedActor {
        subject: approval.subject.clone(),
        grant_id: approval.grant_id.clone(),
        grant_epoch: approval.grant_epoch,
    };
    let mut expected = PolicyFence::current(state, &approver, journal);
    // Adoption replaced the reviewed prior pointer; every other fence must remain current.
    expected.expected_active_version = review.fence.expected_active_version;
    if binding.fence != expected {
        return Err(PolicyOwnerError::StaleReview);
    }
    check_prepare_grants(state, actor, &approver, approval, now)?;
    let policy = journal.policy(&binding.target)?.typed()?;
    state.validate_target(&policy)?;
    Ok(AdmittedPolicy {
        binding: binding.clone(),
        approval: approval.clone(),
        approver,
        policy,
    })
}

fn realize_active(
    state: &TrustedPolicyState,
    policy: &MemoryPolicy,
    request: ActivePolicyPreparation,
    now: &str,
) -> Result<RealizedSelection, PolicyOwnerError> {
    let limits = &state.capabilities.effective_limits;
    for (field, requested, effective) in [
        (
            "max_results",
            request.query.limit,
            limits.max_results.min(policy.max_results),
        ),
        (
            "max_candidates",
            request.query.max_candidates,
            limits.max_candidates.min(policy.max_candidates),
        ),
        (
            "max_query_bytes",
            request.query.query.len(),
            limits.max_query_bytes,
        ),
    ] {
        admit(field, requested, effective)?;
    }
    let realized = realize_policy(
        &state.corpus,
        policy,
        request.selection_id,
        &request.query,
        request.mandatory_bytes,
        request.phase2_prepared_manifest_sha256,
        request.prepared_content_ref,
        request.expires_at,
        now,
    )?;
    admit(
        "max_job_input_bytes",
        realized.selection.whole_rendered_bytes,
        limits.max_job_input_bytes,
    )?;
    Ok(realized)
}

fn admit(field: &str, requested: usize, effective: usize) -> Result<(), PolicyOwnerError> {
    if requested > effective {
        return Err(MemoryError::CapabilityLimitExceeded {
            limit: field.to_owned(),
            requested,
            effective,
        }
        .into());
    }
    Ok(())
}

fn check_prepare_grants(
    state: &TrustedPolicyState,
    selector: &AuthorizedActor,
    approver: &AuthorizedActor,
    approval: &PolicyApproval,
    now: u64,
) -> Result<(), PolicyOwnerError> {
    state.check_actor(selector, PolicyPermission::Select, now)?;
    if approval.expires_at <= now || approval.approved_at > now {
        return Err(PolicyOwnerError::GrantRevoked);
    }
    state.check_actor(approver, PolicyPermission::Approve, now)
}
