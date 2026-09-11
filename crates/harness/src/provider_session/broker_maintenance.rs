// SPDX-License-Identifier: MIT

use super::super::types::*;
use super::ProviderSessionBroker;

impl ProviderSessionBroker {
    #[allow(clippy::too_many_arguments)]
    pub fn plan_fork(
        &mut self,
        owner_token: &str,
        source_binding_id: &str,
        fork_plan_id: &str,
        cutoff_turn_ref: &str,
        cutoff_sequence: u64,
        operation: ForkOperation,
        dependencies: Vec<String>,
    ) -> Result<ForkPlan, SessionError> {
        self.authorize_owner(owner_token)?;
        let required_method = match operation {
            ForkOperation::NativeFork => "thread/fork",
            ForkOperation::CleanRehydration => "thread/start",
        };
        if !self
            .capabilities
            .enabled_methods
            .iter()
            .any(|method| method == required_method)
        {
            return Err(SessionError::Unsupported);
        }
        let source = self
            .bindings
            .get(source_binding_id)
            .ok_or(SessionError::NotFound)?
            .clone();
        let watermark = self
            .histories
            .get(source_binding_id)
            .and_then(|items| items.last().map(|item| item.sequence))
            .unwrap_or(0);
        let cutoff_verified = cutoff_sequence == 0
            || self.histories.get(source_binding_id).is_some_and(|items| {
                items.iter().any(|item| {
                    item.sequence == cutoff_sequence && item.turn_ref == cutoff_turn_ref
                })
            });
        if !matches!(source.state, BindingState::Held | BindingState::Active)
            || cutoff_sequence > watermark
            || !cutoff_verified
            || !valid_id(fork_plan_id)
            || !valid_id(cutoff_turn_ref)
            || dependencies.len() > MAX_DEPENDENCIES
            || !unique_ids(&dependencies)
        {
            return Err(SessionError::InvalidRequest);
        }
        if self.fork_plans.contains_key(fork_plan_id) {
            return Err(SessionError::Conflict);
        }
        let mut dependency_ids = source.dependency_ids.clone();
        for dependency in dependencies {
            if !dependency_ids.contains(&dependency) {
                dependency_ids.push(dependency);
            }
        }
        if dependency_ids.len() > MAX_DEPENDENCIES || !unique_ids(&dependency_ids) {
            return Err(SessionError::Capacity);
        }
        let target_binding_id = self.allocate_id("binding");
        let mut target = SessionBinding::candidate(
            target_binding_id.clone(),
            self.scope.clone(),
            "evaluation",
            SessionPurpose::Evaluation,
            self.policy.credential_realm_ref.clone(),
            self.policy.profile_sha256.clone(),
            source.expires_at.clone(),
        )?;
        target.dependency_ids = dependency_ids.clone();
        target.validate()?;
        self.bindings.insert(target_binding_id.clone(), target);
        let plan = ForkPlan {
            schema: SESSION_FORK_SCHEMA.to_owned(),
            fork_plan_id: fork_plan_id.to_owned(),
            scope: self.scope.clone(),
            source_binding_id: source_binding_id.to_owned(),
            source_history_epoch: source.history_epoch,
            source_continuity_sha256: source.continuity_sha256.clone(),
            cutoff_turn_ref: cutoff_turn_ref.to_owned(),
            cutoff_sequence,
            native_cutoff_verified: matches!(operation, ForkOperation::NativeFork),
            target_binding_id,
            purpose: "evaluation".to_owned(),
            operation,
            copies_native_history: matches!(operation, ForkOperation::NativeFork),
            dependency_ids: dependency_ids.clone(),
            automatic_inference: false,
            game_dispatch_capability: false,
        };
        self.fork_plans
            .insert(fork_plan_id.to_owned(), plan.clone());
        Ok(plan)
    }

    pub fn complete_fork(
        &mut self,
        owner_token: &str,
        fork_plan_id: &str,
        native_thread_ref: &str,
    ) -> Result<SessionBinding, SessionError> {
        self.authorize_owner(owner_token)?;
        if !valid_id(native_thread_ref) {
            return Err(SessionError::InvalidRequest);
        }
        let plan = self
            .fork_plans
            .get(fork_plan_id)
            .ok_or(SessionError::NotFound)?
            .clone();
        let source = self
            .bindings
            .get(&plan.source_binding_id)
            .ok_or(SessionError::NotFound)?;
        if matches!(source.state, BindingState::Retired | BindingState::Closed)
            || source.history_epoch != plan.source_history_epoch
            || source.continuity_sha256 != plan.source_continuity_sha256
        {
            return Err(SessionError::Stale);
        }
        let binding = self
            .bindings
            .get_mut(&plan.target_binding_id)
            .ok_or(SessionError::NotFound)?;
        if matches!(
            binding.state,
            BindingState::Retired | BindingState::Closed | BindingState::Quarantined
        ) {
            return Err(SessionError::Retired);
        }
        if binding.state == BindingState::Held {
            return if binding.native_thread_ref == native_thread_ref {
                Ok(binding.clone())
            } else {
                binding.state = BindingState::Quarantined;
                binding.game_dispatch_capability = false;
                Err(SessionError::Conflict)
            };
        }
        binding.native_thread_ref = native_thread_ref.to_owned();
        binding.state = BindingState::Held;
        binding.purpose = SessionPurpose::Evaluation;
        binding.game_dispatch_capability = false;
        Ok(binding.clone())
    }
}
