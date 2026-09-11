// SPDX-License-Identifier: MIT

use super::super::types::*;
use super::ProviderSessionBroker;

impl ProviderSessionBroker {
    /// Requests interruption without treating a late native result as newly authorized work.
    pub fn interrupt(
        &mut self,
        owner_token: &str,
        operation_id: &str,
    ) -> Result<NativeOperation, SessionError> {
        self.authorize_owner(owner_token)?;
        if !self
            .capabilities
            .enabled_methods
            .iter()
            .any(|method| method == "turn/interrupt")
        {
            return Err(SessionError::Unsupported);
        }
        let (binding_id, unknown) = {
            let operation = self
                .operations
                .get_mut(operation_id)
                .ok_or(SessionError::NotFound)?;
            if operation.kind != NativeOperationKind::Turn {
                return Err(SessionError::Conflict);
            }
            if operation.owner_epoch != self.owner_epoch {
                return Err(SessionError::Stale);
            }
            match operation.state {
                NativeOperationState::IntentPersisted => {
                    operation.state = NativeOperationState::Cancelled;
                    operation.terminal_evidence_ref = Some(format!("cancelled-{operation_id}"));
                    (operation.binding_id.clone(), false)
                }
                NativeOperationState::Sent | NativeOperationState::Acknowledged => {
                    operation.state = NativeOperationState::Unknown;
                    (operation.binding_id.clone(), true)
                }
                NativeOperationState::Completed
                | NativeOperationState::Cancelled
                | NativeOperationState::Rejected
                | NativeOperationState::Unknown
                | NativeOperationState::Quarantined
                | NativeOperationState::Planned => return Err(SessionError::Conflict),
            }
        };
        if unknown {
            if let Some(binding) = self.bindings.get_mut(&binding_id) {
                binding.state = BindingState::Recovering;
                binding.game_dispatch_capability = false;
            }
            self.emit(
                &binding_id,
                Some(operation_id),
                SessionEventKind::OperationUnknown,
                SessionEventStatus::Unknown,
                1,
            );
        }
        self.inflight_turn = None;
        self.operations
            .get(operation_id)
            .cloned()
            .ok_or(SessionError::NotFound)
    }

    /// Quarantines an attempt when incompatible terminal evidence is observed.  A quarantined
    /// result is inspectable but can never release the executable binding.
    pub fn quarantine_operation(
        &mut self,
        owner_token: &str,
        operation_id: &str,
        evidence_ref: &str,
    ) -> Result<NativeOperation, SessionError> {
        self.authorize_owner(owner_token)?;
        if !valid_id(evidence_ref) {
            return Err(SessionError::InvalidRequest);
        }
        let binding_id = {
            let operation = self
                .operations
                .get_mut(operation_id)
                .ok_or(SessionError::NotFound)?;
            if !matches!(
                operation.state,
                NativeOperationState::Completed
                    | NativeOperationState::Rejected
                    | NativeOperationState::Unknown
                    | NativeOperationState::IntentPersisted
            ) {
                return Err(SessionError::Conflict);
            }
            operation.state = NativeOperationState::Quarantined;
            operation.terminal_evidence_ref = Some(evidence_ref.to_owned());
            operation.binding_id.clone()
        };
        if let Some(binding) = self.bindings.get_mut(&binding_id) {
            binding.state = BindingState::Quarantined;
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
        self.operations
            .get(operation_id)
            .cloned()
            .ok_or(SessionError::NotFound)
    }
}
