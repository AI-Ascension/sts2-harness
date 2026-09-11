// SPDX-License-Identifier: MIT

use super::super::types::*;
use super::ProviderSessionBroker;
use serde_json::json;

impl ProviderSessionBroker {
    pub(super) fn prepared_bytes(&self) -> Result<usize, SessionError> {
        self.prepared.values().try_fold(0_usize, |total, prepared| {
            let size = prepared
                .suffix
                .len()
                .checked_add(prepared.output_schema.len())
                .and_then(|value| value.checked_add(prepared.protected.len()))
                .ok_or(SessionError::Capacity)?;
            total.checked_add(size).ok_or(SessionError::Capacity)
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn prepare_turn(
        &mut self,
        owner_token: &str,
        binding_id: &str,
        prepared_id: &str,
        phase2_preview_id: &str,
        phase2_revision_id: &str,
        phase3_selection_id: &str,
        held_boundary_ref: &str,
        suffix: Vec<u8>,
        output_schema: Vec<u8>,
        protected: Vec<u8>,
        dependencies: Vec<String>,
        expires_at: &str,
    ) -> Result<PreparedSessionTurn, SessionError> {
        self.authorize_owner(owner_token)?;
        if !self
            .capabilities
            .enabled_methods
            .iter()
            .any(|method| method == "turn/start")
        {
            return Err(SessionError::Unsupported);
        }
        let binding = self.ensure_binding_not_expired(binding_id)?;
        if binding.state != BindingState::Held || !self.policy.allows_execution() {
            return Err(SessionError::HeldRequired);
        }
        if self.prepared.contains_key(prepared_id) {
            return Err(SessionError::Conflict);
        }
        if self.prepared.len() >= MAX_PREPARED {
            return Err(SessionError::Capacity);
        }
        let new_prepared_bytes = suffix
            .len()
            .checked_add(output_schema.len())
            .and_then(|value| value.checked_add(protected.len()))
            .ok_or(SessionError::Capacity)?;
        let total_prepared_bytes = self
            .prepared_bytes()?
            .checked_add(new_prepared_bytes)
            .ok_or(SessionError::Capacity)?;
        if total_prepared_bytes > MAX_PREPARED_BYTES {
            return Err(SessionError::Capacity);
        }
        if dependencies
            .iter()
            .any(|id| !binding.dependency_ids.contains(id))
            || binding
                .dependency_ids
                .iter()
                .any(|id| !dependencies.contains(id))
        {
            return Err(SessionError::Stale);
        }
        self.ensure_not_expired(expires_at)?;
        let mut prepared = PreparedSessionTurn::new(
            prepared_id,
            self.scope.clone(),
            &binding,
            phase2_preview_id,
            phase2_revision_id,
            phase3_selection_id,
            held_boundary_ref,
            suffix,
            output_schema,
            protected,
            dependencies,
            self.policy.continuity,
            expires_at,
        )?;
        prepared.auth_epoch = self.owner_epoch;
        prepared.revocation_epoch = self.revocation_epoch;
        prepared.validate()?;
        self.prepared
            .insert(prepared.prepared_id.clone(), prepared.clone());
        Ok(prepared)
    }

    /// Explicit Phase 2 resume handoff.  No native operation is started here; the scheduler must
    /// call this method only after its own commit/permission transaction succeeds.
    pub fn explicit_resume(
        &mut self,
        owner_token: &str,
        binding_id: &str,
    ) -> Result<SessionBinding, SessionError> {
        self.authorize_owner(owner_token)?;
        if !self
            .capabilities
            .enabled_methods
            .iter()
            .any(|method| method == "turn/start")
        {
            return Err(SessionError::Unsupported);
        }
        self.ensure_binding_not_expired(binding_id)?;
        if self
            .bindings
            .values()
            .any(|candidate| candidate.binding_id != binding_id && candidate.executable())
        {
            return Err(SessionError::Conflict);
        }
        let binding = self
            .bindings
            .get_mut(binding_id)
            .ok_or(SessionError::NotFound)?;
        if binding.executable() {
            return Ok(binding.clone());
        }
        if binding.state != BindingState::Held
            || matches!(binding.purpose, SessionPurpose::Evaluation)
            || !self.policy.allows_execution()
        {
            return Err(SessionError::HeldRequired);
        }
        binding.state = BindingState::Active;
        binding.game_dispatch_capability = true;
        Ok(binding.clone())
    }

    pub fn admit_turn(
        &mut self,
        owner_token: &str,
        binding_id: &str,
        prepared_id: &str,
        idempotency_key: &str,
    ) -> Result<NativeOperation, SessionError> {
        self.authorize_owner(owner_token)?;
        if !self
            .capabilities
            .enabled_methods
            .iter()
            .any(|method| method == "turn/start")
        {
            return Err(SessionError::Unsupported);
        }
        let binding = self.ensure_binding_not_expired(binding_id)?;
        let prepared = self
            .prepared
            .get(prepared_id)
            .ok_or(SessionError::NotFound)?;
        self.ensure_not_expired(&prepared.expires_at)?;
        if matches!(binding.state, BindingState::Retired | BindingState::Closed) {
            return Err(SessionError::Retired);
        }
        if binding.state == BindingState::Quarantined {
            return Err(SessionError::Fenced);
        }
        if binding.state != BindingState::Active || !binding.game_dispatch_capability {
            return Err(SessionError::HeldRequired);
        }
        // The approval dependency vector is revalidated at the first resumed submission, not only
        // when the preview was prepared.  Any profile/continuity/history/compaction/authorization
        // drift, dependency change or tampered record invalidates the exact approved bytes.
        if prepared.validate().is_err() {
            return Err(SessionError::InvalidPrepared);
        }
        if prepared.binding_id != binding_id
            || prepared.owner_epoch != self.owner_epoch
            || prepared.auth_epoch != self.owner_epoch
            || prepared.session_epoch != binding.session_epoch
            || prepared.history_epoch != binding.history_epoch
            || prepared.compaction_epoch != binding.compaction_epoch
            || prepared.revocation_epoch != self.revocation_epoch
            || prepared.profile_sha256 != binding.profile_sha256
            || prepared.continuity_sha256 != binding.continuity_sha256
            || prepared.dependency_ids != binding.dependency_ids
        {
            return Err(SessionError::Stale);
        }
        if !valid_id(idempotency_key) {
            return Err(SessionError::InvalidRequest);
        }
        let request = json!({"prepared_id": prepared_id, "suffix_sha256": prepared.suffix_sha256});
        if let Some(existing) = self.existing_idempotent(idempotency_key, &request)? {
            return Ok(existing);
        }
        if self.inflight_turn.is_some() {
            return Err(SessionError::Conflict);
        }
        let operation = self.new_operation(
            binding_id,
            NativeOperationKind::Turn,
            idempotency_key,
            &request,
            true,
        )?;
        self.inflight_turn = Some(operation.operation_id.clone());
        self.idempotency.insert(
            idempotency_key.to_owned(),
            (
                operation.request_sha256.clone(),
                operation.operation_id.clone(),
            ),
        );
        Ok(operation)
    }
}
