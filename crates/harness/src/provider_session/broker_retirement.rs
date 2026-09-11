// SPDX-License-Identifier: MIT

use super::super::types::*;
use super::{BROKER_SNAPSHOT_SCHEMA, BrokerSnapshot, ProviderSessionBroker};

impl ProviderSessionBroker {
    pub fn set_dependencies(
        &mut self,
        owner_token: &str,
        binding_id: &str,
        dependencies: Vec<String>,
    ) -> Result<SessionBinding, SessionError> {
        self.authorize_owner(owner_token)?;
        if dependencies.len() > MAX_DEPENDENCIES || !unique_ids(&dependencies) {
            return Err(SessionError::Capacity);
        }
        let binding = self
            .bindings
            .get_mut(binding_id)
            .ok_or(SessionError::NotFound)?;
        if matches!(binding.state, BindingState::Retired | BindingState::Closed) {
            return Err(SessionError::Retired);
        }
        binding.dependency_ids = dependencies;
        binding.session_epoch = binding.session_epoch.saturating_add(1);
        binding.game_dispatch_capability = false;
        Ok(binding.clone())
    }

    pub fn revoke_sources(
        &mut self,
        owner_token: &str,
        source_ids: Vec<String>,
    ) -> Result<Vec<Retirement>, SessionError> {
        self.authorize_owner(owner_token)?;
        if source_ids.is_empty() || source_ids.len() > MAX_DEPENDENCIES || !unique_ids(&source_ids)
        {
            return Err(SessionError::InvalidRequest);
        }
        let affected: Vec<String> = self
            .bindings
            .values()
            .filter(|binding| {
                !matches!(binding.state, BindingState::Retired | BindingState::Closed)
                    && binding
                        .dependency_ids
                        .iter()
                        .any(|dependency| source_ids.contains(dependency))
            })
            .map(|binding| binding.binding_id.clone())
            .collect();
        self.revocation_epoch = self.revocation_epoch.saturating_add(1);
        let mut retirements = Vec::with_capacity(affected.len());
        for (index, binding_id) in affected.into_iter().enumerate() {
            let retirement_id = format!("revocation-{}-{index}", self.revocation_epoch);
            retirements.push(self.retire(
                owner_token,
                &binding_id,
                &retirement_id,
                source_ids.clone(),
            )?);
        }
        Ok(retirements)
    }

    pub fn replace_owner(
        &mut self,
        owner_token: &str,
        replacement_token: impl Into<String>,
    ) -> Result<u64, SessionError> {
        self.authorize_owner(owner_token)?;
        let replacement_token = replacement_token.into();
        if replacement_token.is_empty() || replacement_token == self.owner_token {
            return Err(SessionError::InvalidRequest);
        }
        self.owner_token = replacement_token;
        self.owner_epoch = self
            .owner_epoch
            .checked_add(1)
            .ok_or(SessionError::Capacity)?;
        self.inflight_turn = None;
        for binding in self.bindings.values_mut() {
            if !matches!(binding.state, BindingState::Retired | BindingState::Closed) {
                binding.owner_epoch = self.owner_epoch;
                if binding.state == BindingState::Active {
                    binding.state = BindingState::Recovering;
                }
                binding.game_dispatch_capability = false;
            }
        }
        Ok(self.owner_epoch)
    }

    pub fn retire(
        &mut self,
        owner_token: &str,
        binding_id: &str,
        retirement_id: &str,
        revoked_sources: Vec<String>,
    ) -> Result<Retirement, SessionError> {
        self.authorize_owner(owner_token)?;
        if !self
            .capabilities
            .enabled_methods
            .iter()
            .any(|method| method == "thread/retire")
        {
            return Err(SessionError::Unsupported);
        }
        if !valid_id(retirement_id)
            || revoked_sources.len() > MAX_DEPENDENCIES
            || !unique_ids(&revoked_sources)
        {
            return Err(SessionError::InvalidRequest);
        }
        if self.retirements.contains_key(retirement_id) {
            return Err(SessionError::Conflict);
        }
        let binding = self
            .bindings
            .get_mut(binding_id)
            .ok_or(SessionError::NotFound)?;
        if matches!(binding.state, BindingState::Retired | BindingState::Closed) {
            return Err(SessionError::Retired);
        }
        binding.state = BindingState::Retired;
        binding.game_dispatch_capability = false;
        self.revocation_epoch = self.revocation_epoch.saturating_add(1);
        let retirement = Retirement {
            schema: SESSION_RETIREMENT_SCHEMA.to_owned(),
            retirement_id: retirement_id.to_owned(),
            scope: self.scope.clone(),
            binding_ids: vec![binding_id.to_owned()],
            revoked_source_ids: revoked_sources,
            admission_denied: true,
            local_status: RetirementLocalStatus::CleanupPending,
            native_cleanup_status: NativeCleanupStatus::Pending,
            remote_erasure_status: RemoteErasureStatus::Unverified,
            cascade_verified: true,
            affected_native_binding_ids: vec![binding.native_thread_ref.clone()],
            auto_resume: false,
        };
        self.retirements
            .insert(retirement_id.to_owned(), retirement.clone());
        self.emit(
            binding_id,
            None,
            SessionEventKind::Retired,
            SessionEventStatus::Denied,
            1,
        );
        Ok(retirement)
    }

    pub fn cleanup(
        &mut self,
        owner_token: &str,
        retirement_id: &str,
    ) -> Result<Retirement, SessionError> {
        self.authorize_owner(owner_token)?;
        let retirement = self
            .retirements
            .get_mut(retirement_id)
            .ok_or(SessionError::NotFound)?;
        for binding_id in &retirement.binding_ids {
            if let Some(binding) = self.bindings.get_mut(binding_id) {
                binding.state = BindingState::Closed;
                binding.game_dispatch_capability = false;
            }
        }
        retirement.local_status = RetirementLocalStatus::Closed;
        retirement.native_cleanup_status = NativeCleanupStatus::Unsupported;
        Ok(retirement.clone())
    }

    /// Crash/restart recovery advances the owner epoch and keeps every binding held.  It never
    /// replays a turn or releases a Phase 2 scheduler latch.
    pub fn recover(&mut self, owner_token: &str) -> Result<u64, SessionError> {
        self.authorize_owner(owner_token)?;
        self.owner_epoch = self
            .owner_epoch
            .checked_add(1)
            .ok_or(SessionError::Capacity)?;
        self.inflight_turn = None;
        for binding in self.bindings.values_mut() {
            if matches!(
                binding.state,
                BindingState::Active | BindingState::Recovering
            ) {
                binding.state = BindingState::Recovering;
                binding.game_dispatch_capability = false;
            }
            if !matches!(binding.state, BindingState::Retired | BindingState::Closed) {
                binding.owner_epoch = self.owner_epoch;
            }
        }
        Ok(self.owner_epoch)
    }

    #[must_use]
    pub fn snapshot(&self) -> BrokerSnapshot {
        BrokerSnapshot {
            schema: BROKER_SNAPSHOT_SCHEMA,
            scope: self.scope.clone(),
            owner_epoch: self.owner_epoch,
            revocation_epoch: self.revocation_epoch,
            policy: self.policy.clone(),
            capabilities: self.capabilities.clone(),
            bindings: self.bindings.values().cloned().collect(),
            operations: self
                .operations
                .values()
                .filter(|operation| {
                    !matches!(
                        operation.kind,
                        NativeOperationKind::Turn | NativeOperationKind::Interrupt
                    )
                })
                .cloned()
                .collect(),
            events: self.events.clone(),
            compaction_jobs: self.compaction_jobs.values().cloned().collect(),
            fork_plans: self.fork_plans.values().cloned().collect(),
            retirements: self.retirements.values().cloned().collect(),
        }
    }

    pub fn snapshot_json(&self) -> Result<Vec<u8>, SessionError> {
        serde_json::to_vec(&self.snapshot()).map_err(|_| SessionError::Protocol)
    }
}
