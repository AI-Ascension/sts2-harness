// SPDX-License-Identifier: MIT

use super::super::types::*;
use super::ProviderSessionBroker;
use serde_json::json;

impl ProviderSessionBroker {
    pub fn create_candidate(
        &mut self,
        owner_token: &str,
        idempotency_key: &str,
        branch_id: &str,
        purpose: SessionPurpose,
        expires_at: &str,
    ) -> Result<NativeOperation, SessionError> {
        self.authorize_owner(owner_token)?;
        if !self.policy.allows_execution() {
            return Err(SessionError::Forbidden);
        }
        if !self
            .capabilities
            .enabled_methods
            .iter()
            .any(|method| method == "thread/start")
        {
            return Err(SessionError::Unsupported);
        }
        if !valid_id(idempotency_key) {
            return Err(SessionError::InvalidRequest);
        }
        let request = json!({"branch_id": branch_id, "purpose": purpose, "expires_at": expires_at});
        if let Some(existing) = self.existing_idempotent(idempotency_key, &request)? {
            return Ok(existing);
        }
        if self
            .bindings
            .values()
            .filter(|binding| {
                matches!(binding.state, BindingState::Candidate)
                    || binding.purpose == SessionPurpose::Evaluation
            })
            .count()
            >= MAX_CANDIDATES
        {
            return Err(SessionError::Capacity);
        }
        let binding_id = self.allocate_id("binding");
        let binding = SessionBinding::candidate(
            binding_id.clone(),
            self.scope.clone(),
            branch_id,
            purpose,
            self.policy.credential_realm_ref.clone(),
            self.policy.profile_sha256.clone(),
            expires_at,
        )?;
        let operation = self.new_operation(
            &binding_id,
            NativeOperationKind::CreateCandidate,
            idempotency_key,
            &request,
            false,
        )?;
        self.bindings.insert(binding_id.clone(), binding);
        self.idempotency.insert(
            idempotency_key.to_owned(),
            (
                operation.request_sha256.clone(),
                operation.operation_id.clone(),
            ),
        );
        self.emit(
            &binding_id,
            Some(&operation.operation_id),
            SessionEventKind::CandidateCreated,
            SessionEventStatus::Observed,
            1,
        );
        Ok(operation)
    }

    pub fn complete_candidate(
        &mut self,
        owner_token: &str,
        operation_id: &str,
        native_thread_ref: &str,
    ) -> Result<SessionBinding, SessionError> {
        self.authorize_owner(owner_token)?;
        if !self
            .capabilities
            .enabled_methods
            .iter()
            .any(|method| method == "thread/start")
        {
            return Err(SessionError::Unsupported);
        }
        let operation = self
            .operations
            .get_mut(operation_id)
            .ok_or(SessionError::NotFound)?;
        if operation.kind == NativeOperationKind::CreateCandidate
            && operation.state == NativeOperationState::Completed
        {
            let binding = self
                .bindings
                .get(&operation.binding_id)
                .ok_or(SessionError::NotFound)?;
            return if binding.native_thread_ref == native_thread_ref {
                Ok(binding.clone())
            } else {
                Err(SessionError::Conflict)
            };
        }
        if operation.kind != NativeOperationKind::CreateCandidate
            || operation.state != NativeOperationState::IntentPersisted
            || operation.owner_epoch != self.owner_epoch
            || !valid_id(native_thread_ref)
        {
            return Err(SessionError::Conflict);
        }
        let binding_id = operation.binding_id.clone();
        let binding = self
            .bindings
            .get_mut(&binding_id)
            .ok_or(SessionError::NotFound)?;
        binding.native_thread_ref = native_thread_ref.to_owned();
        binding.state = BindingState::Held;
        binding.history_coverage = HistoryCoverage::Unknown;
        operation.state = NativeOperationState::Completed;
        operation.terminal_evidence_ref = Some(format!("native-create-{operation_id}"));
        let value = binding.clone();
        self.emit(
            &binding_id,
            Some(operation_id),
            SessionEventKind::CandidateCreated,
            SessionEventStatus::Observed,
            1,
        );
        Ok(value)
    }

    pub fn reconnect(
        &mut self,
        owner_token: &str,
        binding_id: &str,
        idempotency_key: &str,
    ) -> Result<NativeOperation, SessionError> {
        self.authorize_owner(owner_token)?;
        if !self
            .capabilities
            .enabled_methods
            .iter()
            .any(|method| method == "thread/read")
        {
            return Err(SessionError::Unsupported);
        }
        let binding = self
            .bindings
            .get(binding_id)
            .ok_or(SessionError::NotFound)?;
        if matches!(binding.state, BindingState::Retired | BindingState::Closed) {
            return Err(SessionError::Retired);
        }
        if !matches!(
            binding.state,
            BindingState::Held | BindingState::Active | BindingState::Recovering
        ) {
            return Err(SessionError::HeldRequired);
        }
        if !valid_id(idempotency_key) {
            return Err(SessionError::InvalidRequest);
        }
        let request = json!({"binding_id": binding_id, "operation": "reconnect"});
        if let Some(existing) = self.existing_idempotent(idempotency_key, &request)? {
            return Ok(existing);
        }
        let operation = self.new_operation(
            binding_id,
            NativeOperationKind::Reconnect,
            idempotency_key,
            &request,
            false,
        )?;
        self.idempotency.insert(
            idempotency_key.to_owned(),
            (
                operation.request_sha256.clone(),
                operation.operation_id.clone(),
            ),
        );
        if let Some(binding) = self.bindings.get_mut(binding_id) {
            binding.state = BindingState::Recovering;
            binding.game_dispatch_capability = false;
            binding.owner_epoch = self.owner_epoch;
        }
        Ok(operation)
    }

    pub fn complete_reconnect(
        &mut self,
        owner_token: &str,
        operation_id: &str,
        continuity_sha256: &str,
        coverage: HistoryCoverage,
    ) -> Result<SessionBinding, SessionError> {
        self.authorize_owner(owner_token)?;
        if !valid_digest(continuity_sha256) {
            return Err(SessionError::InvalidRequest);
        }
        let operation = self
            .operations
            .get_mut(operation_id)
            .ok_or(SessionError::NotFound)?;
        if operation.kind == NativeOperationKind::Reconnect
            && operation.state == NativeOperationState::Completed
        {
            let binding = self
                .bindings
                .get(&operation.binding_id)
                .ok_or(SessionError::NotFound)?;
            return if binding.continuity_sha256 == continuity_sha256 {
                Ok(binding.clone())
            } else {
                Err(SessionError::Conflict)
            };
        }
        if operation.kind != NativeOperationKind::Reconnect
            || operation.state != NativeOperationState::IntentPersisted
            || operation.owner_epoch != self.owner_epoch
        {
            return Err(SessionError::Conflict);
        }
        let binding = self
            .bindings
            .get_mut(&operation.binding_id)
            .ok_or(SessionError::NotFound)?;
        binding.state = BindingState::Held;
        binding.session_epoch = binding.session_epoch.saturating_add(1);
        binding.owner_epoch = self.owner_epoch;
        binding.continuity_sha256 = continuity_sha256.to_owned();
        binding.history_coverage = coverage;
        operation.state = NativeOperationState::Completed;
        operation.terminal_evidence_ref = Some(format!("reconnect-{operation_id}"));
        let value = binding.clone();
        self.emit(
            &value.binding_id,
            Some(operation_id),
            SessionEventKind::ReconnectedHeld,
            SessionEventStatus::Observed,
            1,
        );
        Ok(value)
    }
}
