// SPDX-License-Identifier: MIT

impl RuntimeV3Port {
    fn validate_current_action(
        &self,
        identity: &ActionIdentity,
        action: &EpisodeLegalAction,
    ) -> Result<(), sts2_harness::PortError> {
        if self.current_state.as_deref() != Some(identity.state_id.as_str())
            || self.generation != identity.generation
            || self
                .current_actions
                .as_ref()
                .and_then(|set| set.find(action.action_id()))
                != Some(action)
        {
            return Err(wire::port_error(
                "action_not_current",
                "dispatch action is not bound to the current host catalog",
                false,
            ));
        }
        Ok(())
    }

    pub(super) fn current_payload(
        &self,
        action: &EpisodeLegalAction,
    ) -> Result<Value, sts2_harness::PortError> {
        let payload = self
            .payloads
            .get(action.action_id())
            .cloned()
            .ok_or_else(|| {
                wire::port_error(
                    "action_payload_missing",
                    "current legal action payload is unavailable",
                    false,
                )
            })?;
        if payload.get("kind").and_then(Value::as_str)
            != Some(wire::action_kind_name(action.kind()))
        {
            return Err(wire::port_error(
                "action_payload_mismatch",
                "legal action kind changed",
                false,
            ));
        }
        Ok(payload)
    }

    fn retain_operation(
        &mut self,
        identity: &ActionIdentity,
        action: &EpisodeLegalAction,
        payload: &Value,
    ) -> Result<(), sts2_harness::PortError> {
        if let Some(existing) = self.operations.get(&identity.operation_id)
            && (existing.action != *action
                || existing.generation != identity.generation
                || existing.state_id != identity.state_id
                || existing.payload != *payload)
        {
            return Err(wire::port_error(
                "operation_conflict",
                "operation identity conflicts",
                false,
            ));
        }
        if self.operations.len() >= MAX_OPERATIONS
            && !self.operations.contains_key(&identity.operation_id)
        {
            return Err(wire::port_error(
                "operation_capacity",
                "operation ledger is full",
                false,
            ));
        }
        let rest_selector = self.rest_selector_value.clone();
        self.operations
            .entry(identity.operation_id.clone())
            .or_insert_with(|| {
                OperationRecord::new(identity, action, payload.clone())
                    .with_rest_selector(rest_selector)
            });
        Ok(())
    }
}

fn legal_action_argument(action_id: &str, payload: Value) -> Value {
    json!({"action_id": action_id, "action": payload})
}
