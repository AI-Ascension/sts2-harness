// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use serde_json::{Value, json};
use sts2_harness::{
    ActionIdentity, ActionKind, DispatchStatus, EpisodeLegalAction, EpisodeLegalActionSet,
    EpisodeObservation, EpisodeStage, RuntimeV4ExpertActionRequest, RuntimeV4ExpertActionResult,
    RuntimeV4ExpertActionStatus, RuntimeV4ExpertObservation, TransitionReceipt, WaitOutcome,
    WaitSample,
};

use super::{RuntimeV3Port, wire};

const PROFILE: &str = "runtime-v4-expert";
const STATE_TOOL: &str = "sts2.expert_state";
const ACTION_TOOL: &str = "sts2.expert_action";
const RECONCILE_TOOL: &str = "sts2.expert_reconcile";
const PROTOCOL_VERSION: &str = "runtime-v4-expert-action";
const PROFILE_NAME: &str = "expert-action";

#[derive(Clone, Debug)]
pub(super) struct ComposedExpertObservation {
    pub(super) observation: EpisodeObservation,
    pub(super) actions: EpisodeLegalActionSet,
    pub(super) payloads: BTreeMap<String, Value>,
}

impl RuntimeV3Port {
    pub(super) fn is_expert_profile(&self) -> bool {
        self.config.runtime_profile == PROFILE
    }

    fn expert_mcp_mut(&mut self) -> Result<&mut super::super::mcp::McpProcess, String> {
        self.expert_mcp
            .as_mut()
            .ok_or_else(|| String::from("expert MCP process is not running"))
    }

    pub(super) fn call_expert_tool(
        &mut self,
        name: &str,
        arguments: Value,
    ) -> Result<(u64, Value), String> {
        let id = self.expert_next_rpc_id;
        self.expert_next_rpc_id = self
            .expert_next_rpc_id
            .checked_add(1)
            .ok_or_else(|| String::from("expert MCP request identity exhausted"))?;
        let response = wire::rpc_call(
            self.expert_mcp_mut()?,
            id,
            "tools/call",
            json!({"name": name, "arguments": arguments}),
        )?;
        let text = response
            .get("result")
            .and_then(|result| result.get("content"))
            .and_then(Value::as_array)
            .and_then(|content| content.first())
            .and_then(|content| content.get("text"))
            .and_then(Value::as_str)
            .ok_or_else(|| format!("MCP tool {name} omitted text content"))?;
        let value: Value = serde_json::from_str(text)
            .map_err(|error| format!("MCP tool {name} returned non-JSON content: {error}"))?;
        if name != STATE_TOOL
            && value.get("correlation_id").and_then(Value::as_str) != Some(id.to_string().as_str())
        {
            return Err(format!("MCP tool {name} returned mismatched correlation"));
        }
        Ok((id, value))
    }

    fn expert_state(&mut self) -> Result<RuntimeV4ExpertObservation, String> {
        let (_, value) = self.call_expert_tool(
            STATE_TOOL,
            json!({
                "instance_id": self.config.instance_id,
                "mcp_session_id": self.config.mcp_session_id
            }),
        )?;
        RuntimeV4ExpertObservation::from_value(value)
            .map_err(|error| format!("Runtime-v4 expert state is invalid: {error}"))
    }

    pub(super) fn compose_current_observation(
        &mut self,
        baseline: EpisodeObservation,
    ) -> Result<EpisodeObservation, String> {
        let expert = self.expert_state()?;
        let normal_actions = self
            .current_actions
            .clone()
            .ok_or_else(|| String::from("normal catalog is unavailable for expert composition"))?;
        let normal_payloads = self.payloads.clone();
        let composed = compose_with_normal(&baseline, &normal_actions, &normal_payloads, &expert)?;
        self.install_composed(&composed);
        Ok(composed.observation)
    }

    /// Rebind a settled Runtime-v3 result to the expert observation before it reaches the
    /// provider. The normal endpoint remains the mutation authority for ordinary actions, while
    /// the expert endpoint supplies the richer postcondition view.
    pub(super) fn compose_receipt_after(
        &mut self,
        receipt: TransitionReceipt,
    ) -> Result<TransitionReceipt, String> {
        let Some(after) = receipt.after().cloned() else {
            return Ok(receipt);
        };
        let after = self.compose_current_observation(after)?;
        Ok(TransitionReceipt::new(
            receipt.operation_id().to_owned(),
            receipt.action().clone(),
            receipt.status(),
            Some(after),
            receipt.effect_kind().map(str::to_owned),
            receipt.error_code().map(str::to_owned),
        ))
    }

    /// Replace the ordinary wait observation with the matching expert observation while keeping
    /// the barrier outcome and effect witness intact.
    pub(super) fn compose_wait_sample(&mut self, sample: WaitSample) -> Result<WaitSample, String> {
        let Some(after) = sample.observation().cloned() else {
            return Ok(sample);
        };
        let after = self.compose_current_observation(after)?;
        let outcome = sample.outcome();
        let effect_kind = sample.effect_kind().map(str::to_owned);
        let mut composed = WaitSample::new(outcome, Some(after));
        if let Some(effect_kind) = effect_kind {
            composed = composed.with_effect_kind(effect_kind);
        }
        Ok(composed)
    }

    pub(super) fn merge_current_expert_actions(
        &mut self,
        state_id: &str,
        generation: u64,
    ) -> Result<(), String> {
        let expert = self.expert_state()?;
        if expert.state_id() != state_id || expert.generation() != generation {
            return Err(String::from(
                "Runtime-v4 expert catalog does not match the Runtime-v3 observation",
            ));
        }
        let normal_actions = self
            .current_actions
            .clone()
            .ok_or_else(|| String::from("normal catalog is unavailable for expert merge"))?;
        let normal_payloads = self.payloads.clone();
        let (actions, payloads) = merge_actions(&normal_actions, &normal_payloads, &expert)?;
        self.current_actions = Some(actions);
        self.payloads = payloads;
        Ok(())
    }

    pub(super) fn dispatch_expert_action(
        &mut self,
        identity: &ActionIdentity,
        action: &EpisodeLegalAction,
        payload: Value,
    ) -> Result<TransitionReceipt, sts2_harness::PortError> {
        let request_value = action_request(&self.config, identity, action, &payload, "request");
        let request = RuntimeV4ExpertActionRequest::from_value(request_value).map_err(|error| {
            wire::port_error("expert_request_invalid", error.to_string(), false)
        })?;
        let (_, value) = self
            .call_expert_tool(
                ACTION_TOOL,
                json!({
                    "instance_id": self.config.instance_id,
                    "mcp_session_id": self.config.mcp_session_id,
                    "lease_id": self.config.lease_id,
                    "lease_epoch": self.config.lease_epoch,
                    "generation": identity.generation,
                    "state_id": identity.state_id,
                    "operation_id": identity.operation_id,
                    "action": {"action_id": action.action_id(), "action": payload}
                }),
            )
            .map_err(|error| wire::port_error("expert_dispatch_failed", error, true))?;
        let result = RuntimeV4ExpertActionResult::from_value(value).map_err(|error| {
            wire::port_error("expert_dispatch_invalid", error.to_string(), false)
        })?;
        validate_expert_result(&result, &request, identity, action)
            .map_err(|error| wire::port_error("expert_dispatch_invalid", error, false))?;
        let receipt = self
            .expert_result_receipt(result, &request, identity, action)
            .map_err(|error| wire::port_error("expert_receipt_invalid", error, false))?;
        super::recording::receipt(&receipt, identity.generation, &self.telemetry);
        Ok(receipt)
    }

    pub(super) fn reconcile_expert_operation(
        &mut self,
        operation_id: &str,
    ) -> Result<TransitionReceipt, String> {
        let record = self
            .operations
            .get(operation_id)
            .cloned()
            .ok_or_else(|| String::from("expert operation is not in the ledger"))?;
        if record.action.kind() != ActionKind::UsePotion {
            return Err(String::from("operation is not a Runtime-v4 expert action"));
        }
        let identity = ActionIdentity::new(
            operation_id.to_owned(),
            record.state_id.clone(),
            record.generation,
            record.action.action_id().to_owned(),
        )
        .map_err(|error| error.to_string())?;
        let request = RuntimeV4ExpertActionRequest::from_value(action_request(
            &self.config,
            &identity,
            &record.action,
            &record.payload,
            "request",
        ))
        .map_err(|error| error.to_string())?;
        let (_, value) = self.call_expert_tool(
            RECONCILE_TOOL,
            json!({
                "instance_id": self.config.instance_id,
                "mcp_session_id": self.config.mcp_session_id,
                "lease_id": self.config.lease_id,
                "lease_epoch": self.config.lease_epoch,
                "operation_id": operation_id
            }),
        )?;
        let result = RuntimeV4ExpertActionResult::from_value(value)
            .map_err(|error| format!("Runtime-v4 expert reconcile response is invalid: {error}"))?;
        validate_expert_result(&result, &request, &identity, &record.action)?;
        self.expert_result_receipt(result, &request, &identity, &record.action)
    }

    pub(super) fn wait_expert_operation(
        &mut self,
        operation_id: &str,
    ) -> Result<WaitSample, String> {
        let receipt = self.reconcile_expert_operation(operation_id)?;
        match receipt.status() {
            DispatchStatus::Settled => {
                Ok(
                    WaitSample::new(WaitOutcome::SameStateMutation, receipt.after().cloned())
                        .with_effect_kind(receipt.effect_kind().ok_or_else(|| {
                            String::from("expert settlement omitted effect witness")
                        })?),
                )
            }
            DispatchStatus::Accepted | DispatchStatus::Unknown => {
                Ok(WaitSample::new(WaitOutcome::Timeout, None))
            }
            DispatchStatus::Rejected | DispatchStatus::Cancelled => {
                Ok(WaitSample::new(WaitOutcome::RecoveryRequired, None))
            }
        }
    }

    fn expert_result_receipt(
        &mut self,
        result: RuntimeV4ExpertActionResult,
        _request: &RuntimeV4ExpertActionRequest,
        identity: &ActionIdentity,
        action: &EpisodeLegalAction,
    ) -> Result<TransitionReceipt, String> {
        let status = match result.status() {
            RuntimeV4ExpertActionStatus::Accepted => DispatchStatus::Accepted,
            RuntimeV4ExpertActionStatus::Settled => DispatchStatus::Settled,
            RuntimeV4ExpertActionStatus::Rejected => DispatchStatus::Rejected,
            RuntimeV4ExpertActionStatus::Unknown => DispatchStatus::Unknown,
            RuntimeV4ExpertActionStatus::Cancelled => DispatchStatus::Cancelled,
        };
        let after = if status == DispatchStatus::Settled {
            let expert =
                RuntimeV4ExpertObservation::from_value(result.as_value()["observation"].clone())
                    .map_err(|error| {
                        format!("expert settlement observation is invalid: {error}")
                    })?;
            let composed = expert_only_observation(&expert)?;
            self.install_composed(&composed);
            Some(composed.observation)
        } else {
            None
        };
        let effect_kind =
            (status == DispatchStatus::Settled).then(|| String::from("potion_use_settled"));
        let error_code = result.as_value()["error_code"].as_str().map(str::to_owned);
        Ok(TransitionReceipt::new(
            identity.operation_id.clone(),
            action.clone(),
            status,
            after,
            effect_kind,
            error_code,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expert() -> RuntimeV4ExpertObservation {
        RuntimeV4ExpertObservation::parse(include_bytes!(
            "../../../../../protocol-artifact/runtime-v4-expert/golden/observation.json"
        ))
        .expect("checked-in expert observation")
    }

    #[test]
    fn composition_keeps_normal_actions_and_adds_only_expert_potions() {
        let expert = expert();
        let play = EpisodeLegalAction::new("play:7:card:1:enemy:1", ActionKind::PlayCard)
            .expect("play action");
        let end = EpisodeLegalAction::new("end:7", ActionKind::EndTurn).expect("end action");
        let normal = EpisodeLegalActionSet::new(
            expert.state_id(),
            expert.generation(),
            vec![play.clone(), end.clone()],
        )
        .expect("normal catalog");
        let payloads = BTreeMap::from([
            (
                play.action_id().to_owned(),
                json!({"kind":"play_card","card_id":"card:1","target_id":"enemy:1"}),
            ),
            (end.action_id().to_owned(), json!({"kind":"end_turn"})),
        ]);
        let (merged, merged_payloads) = merge_actions(&normal, &payloads, &expert).unwrap();
        assert_eq!(merged.actions().len(), 3);
        assert_eq!(merged.actions()[0], play);
        assert_eq!(merged.actions()[1], end);
        assert_eq!(merged.actions()[2].kind(), ActionKind::UsePotion);
        assert_eq!(merged_payloads.len(), 3);
        assert_eq!(
            merged_payloads[merged.actions()[2].action_id()]["kind"],
            "use_potion"
        );
    }

    #[test]
    fn expert_settlement_keeps_parameterized_character_and_rest_actions() -> Result<(), String> {
        let mut value = expert().as_value().clone();
        value["legal_actions"] = json!([
            {
                "action_id": "character:7:ironclad",
                "action": {"kind": "select_character", "character_id": "ironclad"}
            },
            {
                "action_id": "rest-option:7:heal",
                "action": {"kind": "rest_option", "rest_option_id": "heal"}
            }
        ]);
        let expert =
            RuntimeV4ExpertObservation::from_value(value).map_err(|error| error.to_string())?;
        let composed = expert_only_observation(&expert)?;
        assert_eq!(composed.actions.actions().len(), 2);
        assert_eq!(
            composed.actions.actions()[0].kind(),
            ActionKind::SelectCharacter
        );
        assert_eq!(composed.actions.actions()[1].kind(), ActionKind::RestOption);
        assert_eq!(
            composed.payloads["character:7:ironclad"]["character_id"],
            "ironclad"
        );
        assert_eq!(
            composed.payloads["rest-option:7:heal"]["rest_option_id"],
            "heal"
        );
        Ok(())
    }

    #[test]
    fn expert_action_request_uses_the_checked_in_action_artifact() {
        let action = EpisodeLegalAction::new("potion:7:potion:fire:enemy:1", ActionKind::UsePotion)
            .expect("potion action");
        let identity = ActionIdentity::new(
            "episode-action-7-1",
            "live:7",
            7,
            action.action_id().to_owned(),
        )
        .expect("identity");
        let config = super::super::RuntimeConfig {
            gateway_address: String::from("127.0.0.1:15525"),
            gateway_token: String::from("token"),
            mcp_binary: String::from("mcp"),
            runtime_profile: String::from(PROFILE),
            instance_id: String::from("instance-1"),
            caller_id: String::from("harness"),
            session_id: String::from("session-1"),
            lease_id: String::from("lease-1"),
            lease_epoch: 1,
            mcp_session_id: String::from("mcp-session-1"),
            run_id: String::from("run-1"),
            episode_id: String::from("episode-1"),
            trajectory_id: String::from("trajectory-1"),
            trace_id: String::from("trace-1"),
            artifact_id: String::from("artifact-1"),
            wait_for_combat_seconds: 0,
            settlement_timeout_seconds: 30,
        };
        let value = action_request(
            &config,
            &identity,
            &action,
            &json!({"kind":"use_potion","potion_id":"potion:fire","target_id":"enemy:1"}),
            "request-1",
        );
        let request = RuntimeV4ExpertActionRequest::from_value(value).expect("action artifact");
        assert_eq!(request.operation_id(), "episode-action-7-1");
        assert_eq!(request.generation(), 7);
        assert_eq!(request.action_id(), action.action_id());
    }

    #[test]
    fn settled_expert_receipt_installs_the_next_catalog_and_effect_witness() -> Result<(), String> {
        let action = EpisodeLegalAction::new("potion:7:potion:fire:enemy:1", ActionKind::UsePotion)
            .map_err(|error| error.to_string())?;
        let identity = ActionIdentity::new(
            "episode-action-7-1",
            "live:7",
            7,
            action.action_id().to_owned(),
        )
        .map_err(|error| error.to_string())?;
        let config = super::super::RuntimeConfig {
            gateway_address: String::from("127.0.0.1:15525"),
            gateway_token: String::from("token"),
            mcp_binary: String::from("mcp"),
            runtime_profile: String::from(PROFILE),
            instance_id: String::from("instance-1"),
            caller_id: String::from("harness"),
            session_id: String::from("session-1"),
            lease_id: String::from("lease-1"),
            lease_epoch: 1,
            mcp_session_id: String::from("mcp-session-1"),
            run_id: String::from("run-1"),
            episode_id: String::from("episode-1"),
            trajectory_id: String::from("trajectory-1"),
            trace_id: String::from("trace-1"),
            artifact_id: String::from("artifact-1"),
            wait_for_combat_seconds: 0,
            settlement_timeout_seconds: 30,
        };
        let mut port =
            RuntimeV3Port::new_with_telemetry(config, super::super::TelemetryHandle::disabled())?;
        let payload = json!({
            "kind": "use_potion",
            "potion_id": "potion:fire",
            "target_id": "enemy:1"
        });
        let request = RuntimeV4ExpertActionRequest::from_value(action_request(
            &port.config,
            &identity,
            &action,
            &payload,
            "request-1",
        ))
        .map_err(|error| error.to_string())?;

        let mut accepted_value: Value = serde_json::from_str(include_str!(
            "../../../../../protocol-artifact/runtime-v4-expert-action/golden/action-settled.json"
        ))
        .map_err(|error| error.to_string())?;
        accepted_value["generation"] = json!(7);
        accepted_value["state_id"] = json!("live:7");
        accepted_value["status"] = json!("accepted");
        accepted_value["observation"] = Value::Null;
        accepted_value["transition"] = Value::Null;
        accepted_value["error_code"] = Value::Null;
        let accepted = RuntimeV4ExpertActionResult::from_value(accepted_value)
            .map_err(|error| error.to_string())?;
        let accepted_receipt =
            port.expert_result_receipt(accepted, &request, &identity, &action)?;
        assert_eq!(accepted_receipt.status(), DispatchStatus::Accepted);
        assert!(accepted_receipt.after().is_none());

        let mut settled_value: Value = serde_json::from_str(include_str!(
            "../../../../../protocol-artifact/runtime-v4-expert-action/golden/action-settled.json"
        ))
        .map_err(|error| error.to_string())?;
        let mut observation: Value = serde_json::from_str(include_str!(
            "../../../../../protocol-artifact/runtime-v4-expert/golden/observation.json"
        ))
        .map_err(|error| error.to_string())?;
        observation["generation"] = json!(8);
        observation["state_id"] = json!("live:8");
        settled_value["generation"] = json!(8);
        settled_value["state_id"] = json!("live:8");
        settled_value["observation"] = observation;
        let settled = RuntimeV4ExpertActionResult::from_value(settled_value)
            .map_err(|error| error.to_string())?;
        let settled_receipt = port.expert_result_receipt(settled, &request, &identity, &action)?;
        assert_eq!(settled_receipt.status(), DispatchStatus::Settled);
        assert_eq!(settled_receipt.effect_kind(), Some("potion_use_settled"));
        assert_eq!(
            settled_receipt.after().map(EpisodeObservation::generation),
            Some(8)
        );
        let current = port
            .current_actions
            .as_ref()
            .ok_or_else(|| String::from("settlement did not install an expert catalog"))?;
        assert!(current.find(action.action_id()).is_some());
        assert!(
            port.payloads[action.action_id()]
                .get("kind")
                .and_then(Value::as_str)
                == Some("use_potion")
        );
        Ok(())
    }
}

fn validate_expert_result(
    result: &RuntimeV4ExpertActionResult,
    request: &RuntimeV4ExpertActionRequest,
    identity: &ActionIdentity,
    action: &EpisodeLegalAction,
) -> Result<(), String> {
    result
        .matches_request(request)
        .map_err(|error| format!("expert response identity mismatch: {error}"))?;
    if result.operation_id() != identity.operation_id
        || result.as_value()["instance_id"] != request.as_value()["instance_id"]
        || result.as_value()["session_id"] != request.as_value()["session_id"]
        || result.as_value()["lease_id"] != request.as_value()["lease_id"]
        || result.as_value()["lease_epoch"] != request.as_value()["lease_epoch"]
    {
        return Err(String::from(
            "expert response does not match the requested action",
        ));
    }
    if result.status() != RuntimeV4ExpertActionStatus::Settled
        && (result.as_value()["state_id"] != identity.state_id
            || result.as_value()["action"] != request.as_value()["action"]
            || result.generation() != identity.generation)
    {
        return Err(String::from(
            "expert response does not match the requested action",
        ));
    }
    if action.kind() != ActionKind::UsePotion {
        return Err(String::from(
            "expert response action kind is not use_potion",
        ));
    }
    Ok(())
}

fn action_request(
    config: &super::RuntimeConfig,
    identity: &ActionIdentity,
    action: &EpisodeLegalAction,
    payload: &Value,
    correlation_id: &str,
) -> Value {
    json!({
        "protocol_version": PROTOCOL_VERSION,
        "schema_digest": sts2_harness::RUNTIME_V4_EXPERT_ACTION_SCHEMA_DIGEST,
        "provenance": {
            "artifact": sts2_harness::RUNTIME_V4_EXPERT_ACTION_ARTIFACT,
            "source": sts2_harness::RUNTIME_V4_EXPERT_ACTION_SCHEMA_SOURCE,
            "generator": sts2_harness::RUNTIME_V4_EXPERT_ACTION_GENERATOR
        },
        "profile": PROFILE_NAME,
        "correlation_id": correlation_id,
        "instance_id": config.instance_id,
        "session_id": config.session_id,
        "lease_id": config.lease_id,
        "lease_epoch": config.lease_epoch,
        "generation": identity.generation,
        "state_id": identity.state_id,
        "operation_id": identity.operation_id,
        "kind": "action_request",
        "action": {"action_id": action.action_id(), "action": payload},
        "status": null,
        "observation": null,
        "transition": null,
        "error_code": null
    })
}

fn compose_with_normal(
    baseline: &EpisodeObservation,
    normal_actions: &EpisodeLegalActionSet,
    normal_payloads: &BTreeMap<String, Value>,
    expert: &RuntimeV4ExpertObservation,
) -> Result<ComposedExpertObservation, String> {
    if baseline.state_id() != expert.state_id() || baseline.generation() != expert.generation() {
        return Err(String::from(
            "Runtime-v4 expert state does not match the Runtime-v3 observation",
        ));
    }
    let (actions, payloads) = merge_actions(normal_actions, normal_payloads, expert)?;
    let stage = expert_stage(expert.as_value())?;
    if stage != baseline.stage() {
        return Err(String::from(
            "Runtime-v4 expert stage does not match the Runtime-v3 observation",
        ));
    }
    let actionable = stage.is_actionable() && !actions.actions().is_empty();
    let observation = EpisodeObservation::new(
        expert.state_id(),
        expert.generation(),
        stage,
        actionable,
        !stage.is_actionable(),
        actionable,
        expert.as_value().clone(),
    )
    .map_err(|error| format!("expert fair-play observation failed validation: {error}"))?;
    Ok(ComposedExpertObservation {
        observation,
        actions,
        payloads,
    })
}

fn merge_actions(
    normal_actions: &EpisodeLegalActionSet,
    normal_payloads: &BTreeMap<String, Value>,
    expert: &RuntimeV4ExpertObservation,
) -> Result<(EpisodeLegalActionSet, BTreeMap<String, Value>), String> {
    let values = expert
        .as_value()
        .get("legal_actions")
        .and_then(Value::as_array)
        .ok_or_else(|| String::from("expert state omitted legal actions"))?;
    let expert_ids: std::collections::BTreeSet<&str> = values
        .iter()
        .filter_map(|value| value.get("action_id").and_then(Value::as_str))
        .collect();
    if normal_actions
        .actions()
        .iter()
        .any(|action| !expert_ids.contains(action.action_id()))
    {
        return Err(String::from(
            "expert catalog omitted a current Runtime-v3 legal action",
        ));
    }
    let mut actions = Vec::with_capacity(normal_actions.actions().len() + values.len());
    let mut payloads = BTreeMap::new();
    for action in normal_actions.actions() {
        let payload = normal_payloads
            .get(action.action_id())
            .cloned()
            .ok_or_else(|| String::from("normal catalog payload is missing"))?;
        let expert_payload = values
            .iter()
            .find(|value| {
                value.get("action_id").and_then(Value::as_str) == Some(action.action_id())
            })
            .and_then(|value| value.get("action"))
            .ok_or_else(|| String::from("expert catalog payload is missing"))?;
        if expert_payload.get("kind").and_then(Value::as_str)
            != Some(wire::action_kind_name(action.kind()))
        {
            return Err(String::from(
                "expert catalog action kind does not match Runtime-v3",
            ));
        }
        actions.push(action.clone());
        payloads.insert(action.action_id().to_owned(), payload);
    }
    for value in values {
        let Some(action_id) = value.get("action_id").and_then(Value::as_str) else {
            return Err(String::from("expert legal action identity is invalid"));
        };
        let Some(payload) = value.get("action") else {
            return Err(String::from("expert legal action payload is missing"));
        };
        if normal_actions.find(action_id).is_some() {
            continue;
        }
        let kind = payload
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| String::from("expert legal action kind is invalid"))?;
        let kind = expert_action_kind(kind)
            .ok_or_else(|| String::from("expert legal action kind is unsupported"))?;
        let action = EpisodeLegalAction::new(action_id, kind).map_err(|error| error.to_string())?;
        actions.push(action);
        payloads.insert(action_id.to_owned(), payload.clone());
    }
    let actions = EpisodeLegalActionSet::new(expert.state_id(), expert.generation(), actions)
        .map_err(|error| error.to_string())?;
    Ok((actions, payloads))
}

fn expert_only_observation(
    expert: &RuntimeV4ExpertObservation,
) -> Result<ComposedExpertObservation, String> {
    let values = expert
        .as_value()
        .get("legal_actions")
        .and_then(Value::as_array)
        .ok_or_else(|| String::from("expert state omitted legal actions"))?;
    let mut actions = Vec::new();
    let mut payloads = BTreeMap::new();
    for value in values {
        let action_id = value
            .get("action_id")
            .and_then(Value::as_str)
            .ok_or_else(|| String::from("expert legal action identity is invalid"))?;
        let payload = value
            .get("action")
            .cloned()
            .ok_or_else(|| String::from("expert legal action payload is missing"))?;
        let Some(kind) = payload.get("kind").and_then(Value::as_str) else {
            return Err(String::from("expert legal action kind is invalid"));
        };
        let Some(kind) = expert_action_kind(kind) else {
            return Err(String::from("expert legal action kind is unsupported"));
        };
        let action = EpisodeLegalAction::new(action_id, kind).map_err(|error| error.to_string())?;
        actions.push(action);
        payloads.insert(action_id.to_owned(), payload);
    }
    let stage = expert_stage(expert.as_value())?;
    let actions = EpisodeLegalActionSet::new(expert.state_id(), expert.generation(), actions)
        .map_err(|error| error.to_string())?;
    let actionable = stage.is_actionable() && !actions.actions().is_empty();
    let observation = EpisodeObservation::new(
        expert.state_id(),
        expert.generation(),
        stage,
        actionable,
        !stage.is_actionable(),
        actionable,
        expert.as_value().clone(),
    )
    .map_err(|error| format!("expert settlement observation failed validation: {error}"))?;
    Ok(ComposedExpertObservation {
        observation,
        actions,
        payloads,
    })
}

fn expert_action_kind(kind: &str) -> Option<ActionKind> {
    Some(match kind {
        "start_run" => ActionKind::StartRun,
        "select_character" => ActionKind::SelectCharacter,
        "select_map_node" => ActionKind::SelectMapNode,
        "play_card" => ActionKind::PlayCard,
        "use_potion" => ActionKind::UsePotion,
        "end_turn" => ActionKind::EndTurn,
        "choose_reward" => ActionKind::ChooseReward,
        "skip_reward" => ActionKind::SkipReward,
        "proceed" => ActionKind::Proceed,
        "confirm_selection" => ActionKind::ConfirmSelection,
        "cancel_selection" => ActionKind::CancelSelection,
        "shop_purchase" => ActionKind::ShopPurchase,
        "shop_remove" => ActionKind::ShopRemove,
        "rest" => ActionKind::Rest,
        "rest_option" => ActionKind::RestOption,
        "smith" => ActionKind::Smith,
        "event_choice" => ActionKind::EventChoice,
        "select_card" => ActionKind::SelectCard,
        "confirm_victory" => ActionKind::ConfirmVictory,
        "save_quit" => ActionKind::SaveQuit,
        _ => return None,
    })
}

fn expert_stage(value: &Value) -> Result<EpisodeStage, String> {
    match value
        .get("state")
        .and_then(Value::as_object)
        .and_then(|state| state.get("state"))
        .and_then(Value::as_str)
    {
        Some("setup") => Ok(EpisodeStage::Setup),
        Some("map") => Ok(EpisodeStage::Map),
        Some("combat") => Ok(EpisodeStage::Combat),
        Some("reward") => Ok(EpisodeStage::Reward),
        Some("shop") => Ok(EpisodeStage::Shop),
        Some("event") => Ok(EpisodeStage::Event),
        Some("rest") => Ok(EpisodeStage::Rest),
        Some("selection") => Ok(EpisodeStage::Selection),
        Some("victory") => Ok(EpisodeStage::Victory),
        Some("defeat") => Ok(EpisodeStage::Defeat),
        Some("recovery") => Ok(EpisodeStage::Recovery),
        _ => Err(String::from("expert state kind is unknown")),
    }
}

impl RuntimeV3Port {
    fn install_composed(&mut self, composed: &ComposedExpertObservation) {
        self.generation = composed.observation.generation();
        self.current_state = Some(composed.observation.state_id().to_owned());
        self.current_actions = Some(composed.actions.clone());
        self.payloads = composed.payloads.clone();
    }
}
