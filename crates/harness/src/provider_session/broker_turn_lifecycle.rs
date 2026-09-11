// SPDX-License-Identifier: MIT

use super::super::types::*;
use super::ProviderSessionBroker;

impl ProviderSessionBroker {
    pub fn mark_sent(&mut self, owner_token: &str, operation_id: &str) -> Result<(), SessionError> {
        self.authorize_owner(owner_token)?;
        let operation = self
            .operations
            .get_mut(operation_id)
            .ok_or(SessionError::NotFound)?;
        if operation.kind != NativeOperationKind::Turn
            || operation.state != NativeOperationState::IntentPersisted
            || operation.owner_epoch != self.owner_epoch
        {
            return Err(SessionError::Stale);
        }
        operation.state = NativeOperationState::Sent;
        Ok(())
    }

    pub fn acknowledge(
        &mut self,
        owner_token: &str,
        operation_id: &str,
        native_turn_ref: &str,
    ) -> Result<(), SessionError> {
        self.authorize_owner(owner_token)?;
        if !valid_id(native_turn_ref) {
            return Err(SessionError::InvalidRequest);
        }
        let operation = self
            .operations
            .get_mut(operation_id)
            .ok_or(SessionError::NotFound)?;
        if operation.kind != NativeOperationKind::Turn
            || !matches!(
                operation.state,
                NativeOperationState::Sent | NativeOperationState::Acknowledged
            )
            || operation.owner_epoch != self.owner_epoch
        {
            return Err(SessionError::Conflict);
        }
        operation.terminal_evidence_ref = Some(native_turn_ref.to_owned());
        operation.state = NativeOperationState::Acknowledged;
        Ok(())
    }

    pub fn complete_turn(
        &mut self,
        owner_token: &str,
        operation_id: &str,
        native_turn_ref: &str,
        item: HistoryItem,
    ) -> Result<NativeOperation, SessionError> {
        self.authorize_owner(owner_token)?;
        if !valid_id(native_turn_ref) || item.validate().is_err() {
            return Err(SessionError::InvalidRequest);
        }
        let binding_id = self
            .operations
            .get(operation_id)
            .ok_or(SessionError::NotFound)?
            .binding_id
            .clone();
        if let Some(existing) = self.operations.get(operation_id)
            && existing.state == NativeOperationState::Completed
        {
            if existing.terminal_evidence_ref.as_deref() == Some(native_turn_ref) {
                return Ok(existing.clone());
            }
            let _ = self.quarantine_operation(owner_token, operation_id, native_turn_ref)?;
            return Err(SessionError::Conflict);
        }
        let completed_turns = self.histories.get(&binding_id).map_or(0, |items| {
            items
                .iter()
                .filter(|item| item.kind == HistoryItemKind::ValidatedDecision)
                .count()
        });
        if completed_turns >= self.policy.max_completed_turns {
            return Err(SessionError::Capacity);
        }
        let operation = self
            .operations
            .get_mut(operation_id)
            .ok_or(SessionError::NotFound)?;
        if operation.kind != NativeOperationKind::Turn
            || !matches!(
                operation.state,
                NativeOperationState::Acknowledged | NativeOperationState::Sent
            )
        {
            return Err(SessionError::Conflict);
        }
        if operation.owner_epoch != self.owner_epoch {
            return Err(SessionError::Stale);
        }
        operation.state = NativeOperationState::Completed;
        operation.terminal_evidence_ref = Some(native_turn_ref.to_owned());
        self.histories.entry(binding_id).or_default().push(item);
        self.inflight_turn = None;
        Ok(operation.clone())
    }

    pub fn mark_unknown(
        &mut self,
        owner_token: &str,
        operation_id: &str,
    ) -> Result<Reconciliation, SessionError> {
        self.authorize_owner(owner_token)?;
        let operation = self
            .operations
            .get_mut(operation_id)
            .ok_or(SessionError::NotFound)?;
        if !matches!(
            operation.state,
            NativeOperationState::IntentPersisted
                | NativeOperationState::Sent
                | NativeOperationState::Acknowledged
        ) {
            return Err(SessionError::Conflict);
        }
        operation.state = NativeOperationState::Unknown;
        let binding_id = operation.binding_id.clone();
        if let Some(binding) = self.bindings.get_mut(&binding_id) {
            binding.state = BindingState::Recovering;
            binding.game_dispatch_capability = false;
        }
        self.inflight_turn = None;
        self.emit(
            &binding_id,
            Some(operation_id),
            SessionEventKind::OperationUnknown,
            SessionEventStatus::Unknown,
            1,
        );
        Ok(Reconciliation {
            schema: SESSION_RECONCILIATION_SCHEMA.to_owned(),
            reconciliation_id: self.allocate_id("reconcile"),
            operation_id: operation_id.to_owned(),
            binding_id,
            scope: self.scope.clone(),
            outcome: ReconciliationOutcome::Ambiguous,
            evidence_refs: Vec::new(),
            native_turn_ref: None,
            auto_retry_generation: false,
            scheduler_after: "held",
            new_generation_calls: 0,
        })
    }
}
