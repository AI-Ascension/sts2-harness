// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use serde_json::{Value, json};
use sts2_harness::{
    ActionIdentity, EpisodeLegalAction, EpisodeLegalActionSet, EpisodeObservation,
    EpisodeRuntimePort, OperationState, ShutdownPort, TransitionReceipt,
};

use super::super::mcp::validate_or_release_allocation_with;
use super::super::runtime_v3_telemetry::ObservationSource;
use super::durable::{OperationCatalogEvidence, OperationIntentEvidence};
use super::{OperationRecord, RuntimeV3Port, allocation_context, parse, wire};

const MAX_OPERATIONS: usize = 1_024;

#[path = "runtime_v3_episode_actions.rs"]
mod actions;
#[cfg(test)]
#[path = "runtime_v3_episode_tests.rs"]
mod tests;

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
            let close = self.close_mcp().map_err(|_| "MCP close failed".to_owned());
            let release = self.release_lease_inner();
            return Err(wire::port_error(
                "runtime_resume_failed",
                wire::combine_cleanup(error, close, release),
                false,
            ));
        }
        Ok(())
    }

    fn observe(&mut self) -> Result<EpisodeObservation, sts2_harness::PortError> {
        let arguments = self.context(self.generation);
        let (value, response_text) = self
            .call_tool("sts2.observe", arguments)
            .map_err(|error| wire::port_error("observe_failed", error, false))?;
        let parsed =
            parse::observation_with_text(&value, &response_text, "state_response", &self.config)
                .map_err(|error| wire::port_error("observe_invalid", error, false))?;
        if let Some(durable) = &self.durable {
            durable
                .verify_resume_boundary_with_catalog(&parsed.observation, &parsed.catalog_raw)
                .map_err(|error| wire::port_error("resume_boundary_mismatch", error, false))?;
        }
        let observation = self
            .install(parsed)
            .map_err(|error| wire::port_error("observe_durability_failed", error, false))?;
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
        let (value, response_text) = self
            .call_tool("sts2.legal_actions", arguments)
            .map_err(|error| wire::port_error("legal_actions_failed", error, false))?;
        if wire::catalog_reobserve(&value) {
            return Err(wire::port_error(
                "catalog_reobserve",
                "host requires a fresh observation before reading legal actions",
                true,
            ));
        }
        let parsed = parse::action_set_with_catalog_text(
            &value,
            &response_text,
            "legal_actions_response",
            &self.config,
        )
        .map_err(|error| wire::port_error("legal_actions_invalid", error, false))?;
        let actions = parsed.actions;
        self.generation = actions.generation();
        self.current_state = Some(actions.state_id().to_owned());
        self.current_actions = Some(actions.clone());
        self.catalog = Some(parsed.catalog);
        self.catalog_raw = Some(parsed.catalog_raw);
        self.payloads = parsed.payloads;
        Ok(actions)
    }

    fn dispatch_action(
        &mut self,
        identity: &ActionIdentity,
        action: &EpisodeLegalAction,
    ) -> Result<TransitionReceipt, sts2_harness::PortError> {
        self.validate_current_action(identity, action)?;
        let payload = self.current_payload(action)?;
        let payload_digest = self.retain_operation(identity, action)?;
        if let Some(durable) = &self.durable {
            let input = json!({
                "state_id": identity.state_id,
                "generation": identity.generation,
                "legal_actions": self.catalog.clone().ok_or_else(|| {
                    wire::port_error(
                        "catalog_missing",
                        "operation intent has no exact legal-action catalog",
                        false,
                    )
                })?,
            });
            let authority = self.recovery_authority.as_ref().ok_or_else(|| {
                wire::port_error(
                    "operation_context_missing",
                    "validated allocation recovery authority is required before dispatch",
                    false,
                )
            })?;
            let original_context_raw =
                super::recovery::RecoveryContext::original_context_raw(authority)
                    .map_err(|error| wire::port_error("operation_context_failed", error, false))?;
            durable
                .persist_operation_intent(OperationIntentEvidence {
                    operation_id: &identity.operation_id,
                    state_id: &identity.state_id,
                    generation: identity.generation,
                    action,
                    payload: &payload,
                    catalog: OperationCatalogEvidence {
                        input: &input,
                        raw: self.catalog_raw.as_deref().ok_or_else(|| {
                            wire::port_error(
                                "catalog_missing",
                                "operation intent has no retained legal-action bytes",
                                false,
                            )
                        })?,
                    },
                    original_context_raw: Some(&original_context_raw),
                })
                .map_err(|error| wire::port_error("operation_intent_failed", error, false))?;
            durable
                .operation_dispatched(&identity.operation_id, &payload_digest)
                .map_err(|error| wire::port_error("dispatch_intent_failed", error, false))?;
        }
        let (value, response_text) = match self.call_tool(
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
        ) {
            Ok(value) => value,
            Err(error) => {
                if let Some(durable) = &self.durable {
                    durable
                        .operation_result(
                            &identity.operation_id,
                            &payload_digest,
                            OperationState::Unknown,
                            None,
                        )
                        .map_err(|durability_error| {
                            wire::port_error("dispatch_durability_failed", durability_error, false)
                        })?;
                }
                return Err(wire::port_error("dispatch_failed", error, true));
            }
        };
        let receipt = match parse::receipt(
            &value,
            &response_text,
            "dispatch_action_response",
            &self.config,
            &identity.operation_id,
            identity.generation,
            action.clone(),
        ) {
            Ok(receipt) => receipt,
            Err(error) => {
                if let Some(durable) = &self.durable {
                    durable
                        .operation_result(
                            &identity.operation_id,
                            &payload_digest,
                            OperationState::Unknown,
                            None,
                        )
                        .map_err(|durability_error| {
                            wire::port_error("dispatch_durability_failed", durability_error, false)
                        })?;
                }
                return Err(wire::port_error("dispatch_invalid", error, false));
            }
        };
        if let Err(error) =
            self.install_response(&value, &response_text, "dispatch_action_response")
        {
            if let Some(durable) = &self.durable {
                durable
                    .operation_result(
                        &identity.operation_id,
                        &payload_digest,
                        OperationState::Unknown,
                        None,
                    )
                    .map_err(|durability_error| {
                        wire::port_error("dispatch_durability_failed", durability_error, false)
                    })?;
            }
            return Err(wire::port_error(
                "dispatch_observation_invalid",
                error,
                false,
            ));
        }
        if let Some(durable) = &self.durable {
            let state = match receipt.status() {
                sts2_harness::DispatchStatus::Accepted => OperationState::Accepted,
                sts2_harness::DispatchStatus::Settled => OperationState::Settled,
                sts2_harness::DispatchStatus::Rejected
                | sts2_harness::DispatchStatus::Cancelled => OperationState::Rejected,
                sts2_harness::DispatchStatus::Unknown => OperationState::Unknown,
            };
            durable
                .operation_result(
                    &identity.operation_id,
                    &payload_digest,
                    state,
                    (state == OperationState::Settled || state == OperationState::Rejected)
                        .then_some(&value),
                )
                .map_err(|error| wire::port_error("dispatch_durability_failed", error, false))?;
        }
        super::recording::receipt(&receipt, identity.generation, &self.telemetry);
        Ok(receipt)
    }
}

fn legal_action_argument(action_id: &str, payload: Value) -> Value {
    json!({"action_id": action_id, "action": payload})
}
