// SPDX-License-Identifier: MIT

impl RuntimeV3Port {
    fn historical_recovery_enabled(&self) -> bool {
        self.recovery_authority.is_some()
    }

    pub(super) fn reconcile_pending_operations(&mut self) -> Result<(), String> {
        let Some(durable) = self.durable.clone() else {
            return Ok(());
        };
        let pending = durable.pending_operations()?;
        for operation in pending {
            let action_bytes = operation.intent.action_payload.as_deref().ok_or_else(|| {
                durable.mark_interrupted_unknown(
                    "legacy operation has no canonical action payload; recovery is blocked",
                );
                format!(
                    "cannot resume operation {} without its canonical action payload",
                    operation.intent.operation_id
                )
            })?;
            let payload: Value = serde_json::from_slice(action_bytes).map_err(|_| {
                durable.mark_interrupted_unknown(
                    "durable operation canonical action payload is malformed",
                );
                format!(
                    "cannot resume operation {} with malformed canonical action payload",
                    operation.intent.operation_id
                )
            })?;
            let envelope = payload.as_object().ok_or_else(|| {
                durable.mark_interrupted_unknown(
                    "durable operation canonical action payload is not an object",
                );
                format!(
                    "cannot resume operation {} with a non-object canonical action payload",
                    operation.intent.operation_id
                )
            })?;
            if envelope.len() != 2
                || envelope.get("action_id").and_then(Value::as_str)
                    != Some(operation.intent.action_id.as_str())
            {
                durable.mark_interrupted_unknown(
                    "durable operation canonical action identity is inconsistent",
                );
                return Err(format!(
                    "cannot resume operation {} with inconsistent canonical action identity",
                    operation.intent.operation_id
                ));
            }
            let action_payload = envelope.get("action").ok_or_else(|| {
                durable.mark_interrupted_unknown(
                    "durable operation canonical action payload is incomplete",
                );
                format!(
                    "cannot resume operation {} with incomplete canonical action payload",
                    operation.intent.operation_id
                )
            })?;
            let action = super::parse::action_from_payload(
                &operation.intent.action_id,
                action_payload,
            )
            .map_err(|error| {
                durable.mark_interrupted_unknown(
                    "durable operation canonical action payload failed validation",
                );
                format!(
                    "cannot resume operation {} with invalid canonical action payload: {error}",
                    operation.intent.operation_id
                )
            })?;
            if operation.intent.action_kind.as_deref()
                != Some(super::wire::action_kind_name(action.kind()))
                || super::wire::canonical_action_digest(action.action_id(), action_payload)
                    .map_err(|error| {
                        durable.mark_interrupted_unknown(
                            "durable operation canonical action digest could not be calculated",
                        );
                        format!(
                            "cannot validate operation {} canonical action: {error}",
                            operation.intent.operation_id
                        )
                    })?
                    != operation.intent.payload_digest
            {
                durable.mark_interrupted_unknown(
                    "durable operation canonical action kind or digest does not match",
                );
                return Err(format!(
                    "cannot resume operation {} with mismatched canonical action identity",
                    operation.intent.operation_id
                ));
            }
            self.operations.insert(
                operation.intent.operation_id.clone(),
                super::ledger::OperationRecord {
                    state_id: operation.intent.state_id.clone(),
                    generation: operation.intent.generation,
                    action,
                    payload: action_payload.clone(),
                    rest_selector: None,
                },
            );
            let operation_id = operation.intent.operation_id.as_str();
            let receipt = self.reconcile(operation_id).map_err(|error| {
                format!("cannot reconcile retained operation {operation_id}: {error}")
            })?;
            if matches!(
                receipt.status(),
                DispatchStatus::Accepted | DispatchStatus::Unknown
            ) {
                return Err(format!(
                    "retained operation {operation_id} remains unresolved"
                ));
            }
            let state = durable.operation_state(operation_id)?;
            if state.is_unresolved() {
                return Err(format!(
                    "retained operation {operation_id} remains in durable state {state:?}"
                ));
            }
        }
        durable.refresh_resume_boundary()?;
        Ok(())
    }
}
