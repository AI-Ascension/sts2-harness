// SPDX-License-Identifier: MIT

use super::types::*;
use crate::context_memory::{MemoryCapabilities, MemoryCorpus, MemoryPolicy};
use crate::management::Authenticator;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

/// Time is consulted at entry and just before publication; a lease does not freeze time.
pub trait PolicyClock: Send + Sync {
    fn now_seconds(&self) -> u64;
    fn now_timestamp(&self) -> String;
}

/// Independently configured owner truth. It is never decoded from a command or reply.
#[derive(Clone)]
pub struct TrustedPolicyState {
    pub corpus: MemoryCorpus,
    pub capabilities: MemoryCapabilities,
    pub trusted_owner_revision: String,
    pub trusted_adapter_revision: String,
    pub trusted_policy_schema_sha256: String,
    pub phase2_revision_id: String,
    pub control_epoch: u64,
    pub plan_epoch: u64,
    pub owner_epoch: u64,
    pub grants: BTreeMap<String, PolicyGrant>,
}

impl TrustedPolicyState {
    pub fn validate(&self) -> Result<(), PolicyOwnerError> {
        self.capabilities.validate_against_trusted(
            &self.trusted_owner_revision,
            &self.trusted_adapter_revision,
            &self.trusted_policy_schema_sha256,
        )?;
        if self.capabilities.scope != *self.corpus.scope()
            || !valid_id(&self.phase2_revision_id)
            || [self.control_epoch, self.plan_epoch, self.owner_epoch]
                .iter()
                .any(|value| *value == 0 || *value > 9_007_199_254_740_991)
            || self.grants.len() > MAX_POLICY_GRANTS
        {
            return Err(PolicyOwnerError::ScopeMismatch);
        }
        for (key, grant) in &self.grants {
            if key != &grant.grant_id
                || !valid_id(key)
                || !valid_id(&grant.subject)
                || grant.scope != *self.corpus.scope()
                || grant.epoch == 0
                || grant.epoch > 9_007_199_254_740_991
            {
                return Err(PolicyOwnerError::PermissionDenied);
            }
        }
        Ok(())
    }

    pub(super) fn check_actor(
        &self,
        actor: &AuthorizedActor,
        permission: PolicyPermission,
        now: u64,
    ) -> Result<(), PolicyOwnerError> {
        let grant = self
            .grants
            .get(&actor.grant_id)
            .ok_or(PolicyOwnerError::PermissionDenied)?;
        if grant.revoked || grant.epoch != actor.grant_epoch || grant.expires_at <= now {
            return Err(PolicyOwnerError::GrantRevoked);
        }
        if grant.subject != actor.subject
            || grant.scope != *self.corpus.scope()
            || !grant.permissions.contains(&permission)
        {
            return Err(PolicyOwnerError::PermissionDenied);
        }
        Ok(())
    }

    pub(super) fn validate_target(&self, policy: &MemoryPolicy) -> Result<(), PolicyOwnerError> {
        if !self.capabilities.enabled {
            return Err(crate::context_memory::MemoryError::Disabled.into());
        }
        policy.validate_against_capabilities(&self.corpus, &self.capabilities)?;
        if policy.phase2_revision_id.as_deref() != Some(self.phase2_revision_id.as_str())
            || policy.corpus_generation != self.corpus.generation()
        {
            return Err(PolicyOwnerError::StaleReview);
        }
        Ok(())
    }
}

pub(super) struct AuthorizedActor {
    pub subject: String,
    pub grant_id: String,
    pub grant_epoch: u64,
}

pub(super) struct AuthorizedPolicyState<'a> {
    pub(super) state: MutexGuard<'a, TrustedPolicyState>,
    pub(super) actor: AuthorizedActor,
    pub(super) clock: &'a dyn PolicyClock,
}

/// The concrete lease-owning authority. All relevant mutation uses `update`; callers must not
/// attach an independently mutable corpus/control snapshot and claim distributed atomicity.
pub struct MemoryPolicyAuthority {
    state: Mutex<TrustedPolicyState>,
    authenticator: Arc<dyn Authenticator>,
    clock: Arc<dyn PolicyClock>,
}

impl MemoryPolicyAuthority {
    pub fn new(
        state: TrustedPolicyState,
        authenticator: Arc<dyn Authenticator>,
        clock: Arc<dyn PolicyClock>,
    ) -> Result<Self, PolicyOwnerError> {
        state.validate()?;
        Ok(Self {
            state: Mutex::new(state),
            authenticator,
            clock,
        })
    }

    /// Privileged composition port, not an operator command. Failed updates publish nothing.
    /// Every successful publication, including a no-op, advances the authority-owned epoch.
    /// Old reviews and active bindings require explicit revalidation after maintenance.
    pub fn update(
        &self,
        update: impl FnOnce(&mut TrustedPolicyState) -> Result<(), PolicyOwnerError>,
    ) -> Result<(), PolicyOwnerError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| PolicyOwnerError::Unavailable)?;
        let mut next = state.clone();
        update(&mut next)?;
        next.validate()?;
        if next.corpus.scope() != state.corpus.scope() {
            return Err(PolicyOwnerError::ScopeMismatch);
        }
        // State epochs cannot be rolled back by a maintenance adapter.
        if next.owner_epoch < state.owner_epoch
            || next.control_epoch < state.control_epoch
            || next.plan_epoch < state.plan_epoch
        {
            return Err(PolicyOwnerError::StaleReview);
        }
        if next.grants.keys().collect::<Vec<_>>() != state.grants.keys().collect::<Vec<_>>()
            && next.owner_epoch <= state.owner_epoch
        {
            return Err(PolicyOwnerError::StaleReview);
        }
        for (key, old) in &state.grants {
            if let Some(new) = next.grants.get(key)
                && (new.epoch < old.epoch
                    || (new.epoch == old.epoch
                        && (new.subject != old.subject
                            || new.permissions != old.permissions
                            || new.revoked != old.revoked
                            || new.expires_at != old.expires_at)))
            {
                return Err(PolicyOwnerError::StaleReview);
            }
        }
        // Corpus disable/reset and reconstruction can reuse generation/revocation values.
        // A separate monotonic fence prevents any maintenance A -> B -> A from restoring
        // an old approval. Preserve an explicitly supplied larger trusted epoch.
        let successor = state
            .owner_epoch
            .checked_add(1)
            .filter(|epoch| *epoch <= 9_007_199_254_740_991)
            .ok_or(PolicyOwnerError::Capacity)?;
        next.owner_epoch = next.owner_epoch.max(successor);
        *state = next;
        Ok(())
    }

    pub fn inspect<T>(
        &self,
        read: impl FnOnce(&TrustedPolicyState) -> Result<T, PolicyOwnerError>,
    ) -> Result<T, PolicyOwnerError> {
        let state = self
            .state
            .lock()
            .map_err(|_| PolicyOwnerError::Unavailable)?;
        read(&state)
    }

    pub(super) fn with_authorized<T>(
        &self,
        access: PolicyAccess<'_>,
        permission: PolicyPermission,
        action: impl FnOnce(
            &TrustedPolicyState,
            &AuthorizedActor,
            &dyn PolicyClock,
        ) -> Result<T, PolicyOwnerError>,
    ) -> Result<T, PolicyOwnerError> {
        let authorized = self.lock_authorized(access, permission)?;
        action(&authorized.state, &authorized.actor, authorized.clock)
    }

    pub(super) fn lock_authorized(
        &self,
        access: PolicyAccess<'_>,
        permission: PolicyPermission,
    ) -> Result<AuthorizedPolicyState<'_>, PolicyOwnerError> {
        if access.bearer.is_some_and(|token| token.len() > 4096) {
            return Err(PolicyOwnerError::Unauthenticated);
        }
        if !valid_id(access.grant_id) {
            return Err(PolicyOwnerError::PermissionDenied);
        }
        let context = self
            .authenticator
            .authenticate(access.bearer)
            .map_err(|_| PolicyOwnerError::Unauthenticated)?;
        let state = self
            .state
            .lock()
            .map_err(|_| PolicyOwnerError::Unavailable)?;
        state.validate()?;
        if !context.can_run(&state.corpus.scope().run_id) {
            return Err(PolicyOwnerError::PermissionDenied);
        }
        let grant = state
            .grants
            .get(access.grant_id)
            .ok_or(PolicyOwnerError::PermissionDenied)?;
        let actor = AuthorizedActor {
            subject: context.subject,
            grant_id: grant.grant_id.clone(),
            grant_epoch: grant.epoch,
        };
        // An authenticated workflow wildcard never substitutes for this explicit scoped grant.
        state.check_actor(&actor, permission, self.clock.now_seconds())?;
        Ok(AuthorizedPolicyState {
            state,
            actor,
            clock: self.clock.as_ref(),
        })
    }
}
