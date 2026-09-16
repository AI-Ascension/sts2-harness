// SPDX-License-Identifier: MIT

use super::*;
use authority::{AuthorizedActor, AuthorizedPolicyState};
use std::sync::MutexGuard;

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

/// Authenticated owner snapshot for one selected game-information lookup policy.
/// The binding carries the durable owner fence that must be revalidated before
/// each query or feedback delivery.
#[derive(Clone, Debug)]
pub struct LookupPolicySnapshot {
    pub binding: ActivePolicyBinding,
    pub policy: MemoryPolicy,
    pub corpus: MemoryCorpus,
    pub capabilities: MemoryCapabilities,
}

/// Holds the selected-policy authority and journal lease through one lifecycle
/// authorization boundary. Dropping the guard releases the journal transaction
/// and both owner locks.
pub struct LookupPolicyAuthorityGuard<'a> {
    snapshot: LookupPolicySnapshot,
    _authority: AuthorizedPolicyState<'a>,
    store: MutexGuard<'a, PolicyStore>,
}

impl LookupPolicyAuthorityGuard<'_> {
    pub fn snapshot(&self) -> &LookupPolicySnapshot {
        &self.snapshot
    }
}

impl Drop for LookupPolicyAuthorityGuard<'_> {
    fn drop(&mut self) {
        self.store.end_authority_lease();
    }
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

    /// Loads the currently adopted policy and a bounded clone of its owner
    /// corpus for a read-only lookup session. Caller-supplied policy bytes,
    /// corpus data, or capabilities are never accepted.
    pub fn lookup_snapshot(
        &self,
        access: PolicyAccess<'_>,
        expected_binding: Option<&ActivePolicyBinding>,
    ) -> Result<LookupPolicySnapshot, PolicyOwnerError> {
        self.authority
            .with_authorized(access, PolicyPermission::Select, |state, actor, clock| {
                let mut store = self
                    .store
                    .lock()
                    .map_err(|_| PolicyOwnerError::Unavailable)?;
                store.read_lease(|journal| {
                    let active = admitted_active(journal, state, actor, clock.now_seconds())?;
                    if expected_binding.is_some_and(|expected| expected != &active.binding) {
                        return Err(PolicyOwnerError::StaleReview);
                    }
                    Ok(LookupPolicySnapshot {
                        binding: active.binding,
                        policy: active.policy,
                        corpus: state.corpus.clone(),
                        capabilities: state.capabilities.clone(),
                    })
                })
            })
    }

    /// Rechecks current selector and approver authority, active journal binding,
    /// corpus generation, capability descriptor, and review fence against the
    /// snapshot retained by a live lookup session.
    pub fn revalidate_lookup_snapshot(
        &self,
        access: PolicyAccess<'_>,
        expected: &ActivePolicyBinding,
    ) -> Result<(), PolicyOwnerError> {
        self.lookup_snapshot(access, Some(expected)).map(|_| ())
    }

    /// Acquires a linearizable selected-policy lease for a short lifecycle
    /// boundary. The authenticated selector and approver grants, current
    /// authority state, and durable active journal stay locked until drop.
    pub fn lock_lookup_snapshot(
        &self,
        access: PolicyAccess<'_>,
        expected: &ActivePolicyBinding,
    ) -> Result<LookupPolicyAuthorityGuard<'_>, PolicyOwnerError> {
        let authorized = self
            .authority
            .lock_authorized(access, PolicyPermission::Select)?;
        let mut store = self
            .store
            .lock()
            .map_err(|_| PolicyOwnerError::Unavailable)?;
        let journal = store.begin_authority_lease()?;
        let snapshot = (|| {
            let active = admitted_active(
                &journal,
                &authorized.state,
                &authorized.actor,
                authorized.clock.now_seconds(),
            )?;
            if &active.binding != expected {
                return Err(PolicyOwnerError::StaleReview);
            }
            Ok(LookupPolicySnapshot {
                binding: active.binding,
                policy: active.policy,
                corpus: authorized.state.corpus.clone(),
                capabilities: authorized.state.capabilities.clone(),
            })
        })();
        let snapshot = match snapshot {
            Ok(snapshot) => snapshot,
            Err(error) => {
                store.end_authority_lease();
                return Err(error);
            }
        };
        Ok(LookupPolicyAuthorityGuard {
            snapshot,
            _authority: authorized,
            store,
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
    // Continued approval authority is required even when maintenance also made the binding stale.
    check_prepare_grants(state, actor, &approver, approval, now)?;
    let mut expected = PolicyFence::current(state, &approver, journal);
    // Adoption replaced the reviewed prior pointer; every other fence must remain current.
    expected.expected_active_version = review.fence.expected_active_version;
    if binding.fence != expected {
        return Err(PolicyOwnerError::StaleReview);
    }
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
