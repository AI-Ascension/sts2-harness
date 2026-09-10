// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use serde_json::{Value, json};
use sts2_harness::{
    ActionIdentity, EpisodeLegalAction, EpisodeLegalActionSet, EpisodeObservation,
    EpisodeRuntimePort, TransitionReceipt,
};

use super::super::mcp::validate_or_release_allocation_with;
use super::{OperationRecord, RuntimeV3Port, RuntimeV3ToolError, allocation_context, parse, wire};

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
        let allocation = validate_or_release_allocation_with(
            allocation,
            &self.config,
            allocation_context::validate,
            |headers| {
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
            },
        )
        .map_err(|error| wire::port_error(code, error, false))?;
        allocation.apply_current_lease(&mut self.config);
        self.recovery_authority = allocation.recovery_authority;
        if let Err(error) = self.launch_mcp() {
            return Err(wire::port_error("runtime_launch_failed", error, false));
        }
        if let Err(error) = self.reconcile_pending_operations() {
            let close = self.close_mcp_processes();
            let release = self.release_lease_inner();
            return Err(wire::port_error(
                "runtime_resume_failed",
                wire::combine_cleanup(error, close, release),
                false,
            ));
        }
        if self.config.seed_transport.is_some() {
            if let Err(error) = self.prime_seed_generation() {
                let close = self.close_mcp_processes();
                let release = self.release_lease_inner();
                return Err(wire::port_error(
                    "seeded_run_preflight_failed",
                    wire::combine_cleanup(error, close, release),
                    false,
                ));
            }
            if let Err(error) = self.launch_seeded_run() {
                let close = self.close_mcp_processes();
                let release = self.release_lease_inner();
                return Err(wire::port_error(
                    "seeded_run_failed",
                    wire::combine_cleanup(error, close, release),
                    false,
                ));
            }
        }
        Ok(())
    }

    fn observe(&mut self) -> Result<EpisodeObservation, sts2_harness::PortError> {
        self.observe_inner(false)
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
        let response_text = self.last_response_text.clone().ok_or_else(|| {
            wire::port_error("legal_actions_invalid", "MCP response text missing", false)
        })?;
        let parsed = parse::action_set_with_catalog_text(
            &value,
            &response_text,
            "legal_actions_response",
            &self.config,
        )
        .map_err(|error| wire::port_error("legal_actions_invalid", error, false))?;
        let actions = parsed.actions;
        let payloads = parsed.payloads;
        self.catalog = Some(parsed.catalog);
        self.catalog_raw = Some(parsed.catalog_raw);
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
        let payload_digest = wire::canonical_action_digest(action.action_id(), &payload)
            .map_err(|error| wire::port_error("operation_intent_failed", error, false))?;
        if let Some(durable) = &self.durable {
            let catalog = self.catalog.clone().ok_or_else(|| {
                wire::port_error(
                    "operation_intent_failed",
                    "legal-action catalog is not retained",
                    false,
                )
            })?;
            let catalog_raw = self.catalog_raw.as_deref().ok_or_else(|| {
                wire::port_error(
                    "operation_intent_failed",
                    "legal-action bytes are not retained",
                    false,
                )
            })?;
            let input = json!({
                "state_id": identity.state_id,
                "generation": identity.generation,
                "legal_actions": catalog,
            });
            durable
                .operation_intent_with_catalog(
                    &identity.operation_id,
                    &identity.state_id,
                    identity.generation,
                    action,
                    &payload,
                    super::durable::OperationCatalogEvidence {
                        input: &input,
                        raw: catalog_raw,
                    },
                )
                .map_err(|error| wire::port_error("operation_intent_failed", error, false))?;
            durable
                .operation_dispatched(&identity.operation_id, &payload_digest)
                .map_err(|error| wire::port_error("dispatch_intent_failed", error, false))?;
        }
        if self.uses_expert_transport(action, &payload) {
            return self.dispatch_expert_action(identity, action, payload);
        }
        let (value, response_text) = self
            .call_tool_with_text(
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
            &response_text,
            "dispatch_action_response",
            &self.config,
            &identity.operation_id,
            identity.generation,
            action.clone(),
        )
        .map_err(|error| wire::port_error("dispatch_invalid", error, false))?;
        self.install_response(&value, &response_text, "dispatch_action_response")
            .map_err(|error| wire::port_error("dispatch_observation_invalid", error, false))?;
        let receipt = if self.is_expert_profile() {
            self.compose_receipt_after(receipt).map_err(|error| {
                wire::port_error("expert_dispatch_observation_invalid", error, false)
            })?
        } else {
            receipt
        };
        if let Some(durable) = &self.durable {
            let state = match receipt.status() {
                sts2_harness::DispatchStatus::Accepted => sts2_harness::OperationState::Accepted,
                sts2_harness::DispatchStatus::Settled => sts2_harness::OperationState::Settled,
                sts2_harness::DispatchStatus::Rejected
                | sts2_harness::DispatchStatus::Cancelled => sts2_harness::OperationState::Rejected,
                sts2_harness::DispatchStatus::Unknown => sts2_harness::OperationState::Unknown,
            };
            durable
                .operation_result(
                    &identity.operation_id,
                    &payload_digest,
                    state,
                    (state == sts2_harness::OperationState::Settled
                        || state == sts2_harness::OperationState::Rejected)
                        .then_some(&value),
                )
                .map_err(|error| wire::port_error("dispatch_durability_failed", error, false))?;
        }
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

#[cfg(test)]
mod tests {
    include!("runtime_v3_episode_tests.rs");
}
