// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use serde_json::{Value, json};
use sts2_harness::{
    ActionIdentity, EpisodeLegalAction, EpisodeLegalActionSet, EpisodeObservation,
    EpisodeRuntimePort, TransitionReceipt,
};

use super::super::mcp::validate_allocation;
use super::{OperationRecord, RuntimeV3Port, parse, wire};

const MAX_OPERATIONS: usize = 1_024;

impl EpisodeRuntimePort for RuntimeV3Port {
    fn launch(&mut self) -> Result<(), sts2_harness::PortError> {
        if self.allocated {
            return Err(wire::port_error(
                "duplicate_launch",
                "episode is already allocated",
                false,
            ));
        }
        let allocation = self
            .gateway
            .request(
                "POST",
                "/v1/sessions/allocate",
                &json!({
                    "instance_id": self.config.instance_id,
                    "caller_id": self.config.caller_id,
                    "session_id": self.config.session_id
                }),
                BTreeMap::new(),
            )
            .map_err(|error| wire::port_error("gateway_allocate_failed", error, false))?;
        self.allocated = true;
        if let Err(error) = validate_allocation(&allocation, &self.config) {
            let release = self.release_lease_inner();
            return Err(wire::port_error(
                "gateway_allocate_invalid",
                wire::combine_cleanup(error, Ok(()), release),
                false,
            ));
        }
        if let Err(error) = self.launch_mcp() {
            return Err(wire::port_error("runtime_launch_failed", error, false));
        }
        Ok(())
    }

    fn observe(&mut self) -> Result<EpisodeObservation, sts2_harness::PortError> {
        let arguments = self.context(self.generation);
        let value = self
            .call_tool("sts2.observe", arguments)
            .map_err(|error| wire::port_error("observe_failed", error, false))?;
        let parsed = parse::observation(&value, "state_response", &self.config)
            .map_err(|error| wire::port_error("observe_invalid", error, false))?;
        Ok(self.install(parsed))
    }

    fn legal_actions(
        &mut self,
        state_id: &str,
        generation: u64,
    ) -> Result<EpisodeLegalActionSet, sts2_harness::PortError> {
        let mut arguments = self.context(generation);
        if let Value::Object(object) = &mut arguments {
            object.insert(String::from("state_id"), Value::String(state_id.to_owned()));
        }
        let value = self
            .call_tool("sts2.legal_actions", arguments)
            .map_err(|error| wire::port_error("legal_actions_failed", error, false))?;
        let (actions, payloads) = parse::action_set(&value, "legal_actions_response", &self.config)
            .map_err(|error| wire::port_error("legal_actions_invalid", error, false))?;
        self.generation = actions.generation();
        self.current_state = Some(actions.state_id().to_owned());
        self.current_actions = Some(actions.clone());
        self.payloads = payloads;
        Ok(actions)
    }

    fn dispatch_action(
        &mut self,
        identity: &ActionIdentity,
        action: &EpisodeLegalAction,
    ) -> Result<TransitionReceipt, sts2_harness::PortError> {
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
        let value = self
            .call_tool(
                "sts2.dispatch_action",
                json!({
                    "instance_id": self.config.instance_id,
                    "mcp_session_id": self.config.mcp_session_id,
                    "lease_id": self.config.lease_id,
                    "lease_epoch": self.config.lease_epoch,
                    "generation": identity.generation,
                    "state_id": identity.state_id,
                    "operation_id": identity.operation_id,
                    "action": legal_action_argument(action.action_id(), payload)
                }),
            )
            .map_err(|error| wire::port_error("dispatch_failed", error, true))?;
        let receipt = parse::receipt(
            &value,
            "dispatch_action_response",
            &self.config,
            &identity.operation_id,
            identity.generation,
            action.clone(),
        )
        .map_err(|error| wire::port_error("dispatch_invalid", error, false))?;
        self.install_response(&value, "dispatch_action_response")
            .map_err(|error| wire::port_error("dispatch_observation_invalid", error, false))?;
        Ok(receipt)
    }
}

fn legal_action_argument(action_id: &str, payload: Value) -> Value {
    json!({"action_id": action_id, "action": payload})
}

#[cfg(test)]
mod tests {
    use super::{json, legal_action_argument};
    use serde_json::Value;

    #[test]
    fn dispatch_preserves_the_complete_host_legal_action_reference()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut request: Value = serde_json::from_str(include_str!(
            "../../../../../protocol-artifact/runtime-v3-gameplay/golden/dispatch-action-request.json"
        ))?;
        let schema: Value = serde_json::from_str(include_str!(
            "../../../../../protocol-artifact/runtime-v3-gameplay/schema.json"
        ))?;
        let validator = jsonschema::validator_for(&schema)?;
        let original_action = request["action"].clone();
        let action_id = original_action["action_id"].as_str().ok_or("action ID")?;
        let payload = original_action["action"].clone();
        request["action"] = legal_action_argument(action_id, payload.clone());
        assert_eq!(request["action"], original_action);
        assert!(validator.is_valid(&request));
        request["action"] = payload;
        assert!(
            !validator.is_valid(&request),
            "bare payload must be rejected"
        );
        let card = json!({"kind": "play_card", "card_id": "c1", "target_id": null});
        request["action"] = legal_action_argument("host-card-ref", card.clone());
        assert_eq!(request["action"]["action_id"], "host-card-ref");
        assert_eq!(request["action"]["action"], card);
        assert!(validator.is_valid(&request));
        Ok(())
    }
}
