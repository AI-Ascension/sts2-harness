// SPDX-License-Identifier: MIT

use serde_json::Value;
use sha2::Digest;
use sts2_harness::{ActionIdentity, EpisodeLegalAction};

use super::{MAX_OPERATIONS, OperationRecord, RuntimeV3Port, wire};

impl RuntimeV3Port {
    pub(super) fn validate_current_action(
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

    pub(super) fn retain_operation(
        &mut self,
        identity: &ActionIdentity,
        action: &EpisodeLegalAction,
    ) -> Result<String, sts2_harness::PortError> {
        if let Some(existing) = self.operations.get(&identity.operation_id)
            && (existing.action != *action
                || existing.generation != identity.generation
                || existing.state_id != identity.state_id)
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
        self.operations
            .entry(identity.operation_id.clone())
            .or_insert_with(|| OperationRecord::new(identity, action));
        let payload = self.current_payload(action)?;
        serde_json::to_vec(&payload)
            .map(|bytes| format!("{:x}", sha2::Sha256::digest(bytes)))
            .map_err(|error| wire::port_error("operation_digest_failed", error.to_string(), false))
    }
}
