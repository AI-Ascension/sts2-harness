// SPDX-License-Identifier: MIT

use super::super::types::*;
use super::ProviderSessionBroker;

impl ProviderSessionBroker {
    /// Records an automatic context transform observed after a turn was submitted but that was not
    /// authorized through an explicit compaction job.  The default response is fail-closed: the
    /// attempt evidence is retained, the attempt can never be admitted as verified work, the
    /// binding is held, and no generation, retry or resume is started.
    pub fn fence_automatic_transform(
        &mut self,
        owner_token: &str,
        binding_id: &str,
        transform_ref: &str,
    ) -> Result<SessionEvent, SessionError> {
        self.authorize_owner(owner_token)?;
        if !valid_id(transform_ref) {
            return Err(SessionError::InvalidRequest);
        }
        let binding = self.ensure_binding_not_expired(binding_id)?;
        if matches!(binding.state, BindingState::Retired | BindingState::Closed) {
            return Err(SessionError::Retired);
        }
        if binding.state == BindingState::Quarantined {
            return Err(SessionError::Fenced);
        }
        if let Some(operation_id) = self.inflight_turn.take()
            && let Some(operation) = self.operations.get_mut(&operation_id)
            && operation.binding_id == binding_id
            && matches!(
                operation.state,
                NativeOperationState::IntentPersisted
                    | NativeOperationState::Sent
                    | NativeOperationState::Acknowledged
            )
        {
            operation.state = NativeOperationState::Unknown;
            operation.terminal_evidence_ref = Some(transform_ref.to_owned());
        }
        if let Some(binding) = self.bindings.get_mut(binding_id) {
            binding.state = BindingState::Recovering;
            binding.game_dispatch_capability = false;
            binding.history_epoch = binding.history_epoch.saturating_add(1);
            binding.history_coverage = HistoryCoverage::Unknown;
        }
        self.emit(
            binding_id,
            None,
            SessionEventKind::TransformObserved,
            SessionEventStatus::Denied,
            1,
        );
        self.events.last().cloned().ok_or(SessionError::Conflict)
    }
}
