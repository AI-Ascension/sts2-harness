// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use serde_json::{Value, json};
use sts2_harness::{
    ActionIdentity, EpisodeLegalAction, EpisodeLegalActionSet, EpisodeObservation,
    EpisodeRuntimePort, TransitionReceipt,
};

use super::super::mcp::validate_or_release_allocation;
use super::super::runtime_v3_telemetry::ObservationSource;
use super::{OperationRecord, RuntimeV3Port, RuntimeV3ToolError, parse, wire};

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
        // Allocation may commit even when its response is lost. Own cleanup before sending.
        self.allocated = true;
        let allocation = self.gateway.request(
            "POST",
            "/v1/sessions/allocate",
            &json!({
                "instance_id": self.config.instance_id,
                "caller_id": self.config.caller_id,
                "session_id": self.config.session_id
            }),
            BTreeMap::from([(
                String::from("x-mcp-session-id"),
                self.config.mcp_session_id.clone(),
            )]),
        );
        let code = if allocation.is_err() {
            "gateway_allocate_failed"
        } else {
            "gateway_allocate_invalid"
        };
        validate_or_release_allocation(allocation, &self.config, |headers| {
            let response = self.gateway.request(
                "POST",
                &format!("/v1/instances/{}/release", self.config.instance_id),
                &json!({}),
                headers,
            );
            self.released = response
                .as_ref()
                .is_ok_and(|value| value["status"] == "released");
            response
        })
        .map_err(|error| wire::port_error(code, error, false))?;
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
        let baseline = self.install(parsed);
        let observation = if self.is_expert_profile() {
            self.compose_current_observation(baseline)
                .map_err(|error| wire::port_error("expert_observe_invalid", error, false))?
        } else {
            baseline
        };
        let _ = self
            .telemetry
            .observation(ObservationSource::Observe, &observation);
        Ok(observation)
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
        let value = match self.call_tool_classified("sts2.legal_actions", arguments) {
            Ok(value) => value,
            Err(RuntimeV3ToolError::Transient(error)) => {
                return Err(wire::port_error(
                    "catalog_reobserve",
                    format!("legal-action catalog transport failed: {error}"),
                    true,
                ));
            }
            Err(RuntimeV3ToolError::Terminal(error)) => {
                return Err(wire::port_error("legal_actions_failed", error, false));
            }
        };
        if wire::catalog_reobserve(&value) {
            return Err(wire::port_error(
                "catalog_reobserve",
                "host requires a fresh observation before reading legal actions",
                true,
            ));
        }
        let (actions, payloads) = parse::action_set(&value, "legal_actions_response", &self.config)
            .map_err(|error| wire::port_error("legal_actions_invalid", error, false))?;
        self.generation = actions.generation();
        self.current_state = Some(actions.state_id().to_owned());
        self.current_actions = Some(actions.clone());
        self.payloads = payloads;
        if self.is_expert_profile() {
            let actions = self.expert_catalog(state_id, generation)?;
            if self.is_rest_profile()
                && self
                    .rest_selector_actions
                    .as_ref()
                    .is_some_and(|selector| selector.assert_matches(state_id, generation).is_ok())
            {
                let selector = self.rest_selector_actions.clone().ok_or_else(|| {
                    wire::port_error("rest_selector_invalid", "selector disappeared", false)
                })?;
                self.current_actions = Some(selector.clone());
                self.payloads = self.rest_selector_payloads.clone();
                return Ok(selector);
            }
            return Ok(actions);
        }
        Ok(actions)
    }

    fn map_snapshot(
        &mut self,
        state_id: &str,
        generation: u64,
        _execution_id: sts2_harness::ModelExecutionId,
    ) -> Result<Option<Value>, sts2_harness::PortError> {
        if self.current_state.as_deref() != Some(state_id) || self.generation != generation {
            return Err(wire::port_error(
                "map_snapshot_stale",
                "map snapshot request is not bound to the current observation",
                false,
            ));
        }
        super::runtime_map::snapshot(&self.config, generation)
            .map(Some)
            .map_err(|error| wire::port_error("map_snapshot_failed", error, false))
    }

    fn dispatch_action(
        &mut self,
        identity: &ActionIdentity,
        action: &EpisodeLegalAction,
    ) -> Result<TransitionReceipt, sts2_harness::PortError> {
        self.validate_current_action(identity, action)?;
        let payload = self.current_payload(action)?;
        self.retain_operation(identity, action, &payload)?;
        if self.uses_expert_transport(action, &payload) {
            return self.dispatch_expert_action(identity, action, payload);
        }
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
        let receipt = if self.is_expert_profile() {
            self.compose_receipt_after(receipt).map_err(|error| {
                wire::port_error("expert_dispatch_observation_invalid", error, false)
            })?
        } else {
            receipt
        };
        super::recording::receipt(&receipt, identity.generation, &self.telemetry);
        Ok(receipt)
    }
}

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
        self.operations
            .entry(identity.operation_id.clone())
            .or_insert_with(|| OperationRecord::new(identity, action, payload.clone()));
        Ok(())
    }
}

fn legal_action_argument(action_id: &str, payload: Value) -> Value {
    json!({"action_id": action_id, "action": payload})
}

#[cfg(test)]
mod tests {
    include!("runtime_v3_episode_tests.rs");
}
