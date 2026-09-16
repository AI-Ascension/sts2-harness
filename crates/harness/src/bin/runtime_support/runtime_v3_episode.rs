// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use serde_json::{Value, json};
use sts2_harness::{
    ActionIdentity, EpisodeLegalAction, EpisodeLegalActionSet, EpisodeObservation,
    EpisodeRuntimePort, TransitionReceipt,
};

use super::super::mcp::validate_or_release_allocation_with;
use super::{RuntimeV3Port, RuntimeV3ToolError, allocation_context, ledger, parse, wire};
use ledger::OperationRecord;

const MAX_OPERATIONS: usize = 1_024;

include!("runtime_v3_episode_legal_actions.rs");

#[path = "runtime_v3_episode_launch.rs"]
mod episode_launch;

impl EpisodeRuntimePort for RuntimeV3Port {
    fn launch(&mut self) -> Result<(), sts2_harness::PortError> {
        episode_launch::launch(self)
    }
    fn current_lease_binding(
        &mut self,
    ) -> Result<sts2_harness::RuntimeLeaseBinding, sts2_harness::PortError> {
        self.allocated_lease_binding()
    }
    fn observe(&mut self) -> Result<EpisodeObservation, sts2_harness::PortError> {
        self.observe_inner(false)
    }

    fn observe_projection(
        &mut self,
        reference: &str,
    ) -> Result<EpisodeObservation, sts2_harness::PortError> {
        if reference != "fair-play.live.v1" {
            return Err(wire::port_error(
                "projection_binding_unavailable",
                format!("runtime does not support authored projection {reference}"),
                false,
            ));
        }
        self.observe()
    }

    fn legal_actions(
        &mut self,
        state_id: &str,
        generation: u64,
    ) -> Result<EpisodeLegalActionSet, sts2_harness::PortError> {
        read_runtime_legal_actions(self, state_id, generation)
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
                        original_context: self
                            .recovery_context
                            .as_ref()
                            .map(|context| context.original_context()),
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
        self.record_durable_receipt(&identity.operation_id, &payload_digest, &receipt, &value)
            .map_err(|error| wire::port_error("dispatch_durability_failed", error, false))?;
        super::recording::receipt(&receipt, identity.generation, &self.telemetry);
        Ok(receipt)
    }
}

include!("runtime_v3_episode_helpers.rs");

#[cfg(test)]
mod tests {
    include!("runtime_v3_episode_tests.rs");
}
