// SPDX-License-Identifier: MIT

#[cfg(test)]
mod tests {
    use super::*;

    fn expert() -> Result<RuntimeV4ExpertObservation, String> {
        RuntimeV4ExpertObservation::parse(include_bytes!(
            "../../../../../protocol-artifact/runtime-v4-expert/golden/observation.json"
        ))
        .map_err(|error| error.to_string())
    }

    #[test]
    fn composition_keeps_normal_actions_and_adds_only_expert_potions() -> Result<(), String> {
        let expert = expert()?;
        let end = EpisodeLegalAction::new("end:7", ActionKind::EndTurn)
            .map_err(|error| error.to_string())?;
        let normal = EpisodeLegalActionSet::new(
            expert.state_id(),
            expert.generation(),
            vec![end.clone()],
        )
        .map_err(|error| error.to_string())?;
        let payloads =
            BTreeMap::from([(end.action_id().to_owned(), json!({"kind":"end_turn"}))]);
        let (merged, merged_payloads) = merge_actions(&normal, &payloads, &expert)?;
        assert_eq!(merged.actions().len(), 3);
        assert_eq!(
            merged
                .actions()
                .iter()
                .map(EpisodeLegalAction::action_id)
                .collect::<Vec<_>>(),
            vec![
                "play:7:card:1:enemy:1",
                "potion:7:potion:fire:enemy:1",
                "end:7"
            ]
        );
        assert_eq!(merged_payloads.len(), 3);
        assert_eq!(
            merged_payloads["end:7"]["kind"],
            "end_turn"
        );
        assert_eq!(
            merged_payloads["potion:7:potion:fire:enemy:1"]["kind"],
            "use_potion"
        );
        Ok(())
    }

    #[test]
    fn composition_order_is_admitted_by_managed_exo_request() -> Result<(), String> {
        let expert = expert()?;
        let normal = EpisodeLegalActionSet::new(
            expert.state_id(),
            expert.generation(),
            vec![EpisodeLegalAction::new("end:7", ActionKind::EndTurn)
                .map_err(|error| error.to_string())?],
        )
        .map_err(|error| error.to_string())?;
        let payloads = BTreeMap::from([(String::from("end:7"), json!({"kind":"end_turn"}))]);
        let (merged, _) = merge_actions(&normal, &payloads, &expert)?;
        let legal_action_ids = merged
            .actions()
            .iter()
            .map(|action| action.action_id().to_owned())
            .collect();
        let input = sts2_harness::ManagedRenderInput {
            execution_id: "model-execution-1".to_owned(),
            state_id: expert.state_id().to_owned(),
            generation: expert.generation(),
            observation: expert.as_value().clone(),
            legal_action_ids,
            objective: "bounded managed decision".to_owned(),
            hard_constraints: Vec::new(),
            map_context: None,
        };
        let boundary = sts2_harness::ContextBoundary {
            run_id: "run-1".to_owned(),
            episode_id: "episode-1".to_owned(),
            agent_id: "agent-1".to_owned(),
            state_id: expert.state_id().to_owned(),
            generation: expert.generation(),
            observation_sha256: "a".repeat(64),
            catalog_sha256: "b".repeat(64),
            adapter_revision: sts2_harness::EXO_SOURCE_REVISION.to_owned(),
            model_revision: "model-1".to_owned(),
            configuration_sha256: "c".repeat(64),
            output_schema_sha256: "d".repeat(64),
            controller_epoch: 1,
            gate_epoch: 0,
            control_version: 0,
        };
        let config = sts2_harness::ExoConfig::new(
            sts2_harness::EXO_SOURCE_REVISION,
            64 * 1024,
            8 * 1024,
            1_000,
        )
        .map_err(|error| error.to_string())?;
        sts2_harness::ContextRenderer::enabled_at_with_limits(
            &boundary,
            input,
            &sts2_harness::ContextDraft::new("draft-1", "revision-1"),
            &BTreeMap::new(),
            &config,
            1,
            &sts2_harness::ContextRenderLimits::harness_maxima(),
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    }

    #[test]
    fn composition_refuses_missing_normal_action() -> Result<(), String> {
        let mut value = expert()?.as_value().clone();
        value["legal_actions"] = json!([
            {
                "action_id": "play:7:card:1:enemy:1",
                "action": {"kind": "play_card", "card_id": "card:1", "target_id": "enemy:1"}
            },
            {
                "action_id": "potion:7:potion:fire:enemy:1",
                "action": {"kind": "use_potion", "potion_id": "potion:fire", "target_id": "enemy:1"}
            }
        ]);
        let expert =
            RuntimeV4ExpertObservation::from_value(value).map_err(|error| error.to_string())?;
        let normal = EpisodeLegalActionSet::new(
            expert.state_id(),
            expert.generation(),
            vec![EpisodeLegalAction::new("end:7", ActionKind::EndTurn)
                .map_err(|error| error.to_string())?],
        )
        .map_err(|error| error.to_string())?;
        let payloads = BTreeMap::from([(String::from("end:7"), json!({"kind":"end_turn"}))]);
        let error = match merge_actions(&normal, &payloads, &expert) {
            Ok(_) => return Err(String::from(
                "expert catalog omitting normal action unexpectedly succeeded",
            )),
            Err(error) => error,
        };
        assert!(error.contains("omitted a current Runtime-v3 legal action"));
        Ok(())
    }

    #[test]
    fn composition_refuses_kind_mismatch_for_normal_action() -> Result<(), String> {
        let mut value = expert()?.as_value().clone();
        value["legal_actions"][2]["action"] =
            json!({"kind":"play_card","card_id":"card:1","target_id":"enemy:1"});
        let expert =
            RuntimeV4ExpertObservation::from_value(value).map_err(|error| error.to_string())?;
        let normal = EpisodeLegalActionSet::new(
            expert.state_id(),
            expert.generation(),
            vec![EpisodeLegalAction::new("end:7", ActionKind::EndTurn)
                .map_err(|error| error.to_string())?],
        )
        .map_err(|error| error.to_string())?;
        let payloads = BTreeMap::from([(String::from("end:7"), json!({"kind":"end_turn"}))]);
        let error = match merge_actions(&normal, &payloads, &expert) {
            Ok(_) => {
                return Err(String::from(
                    "expert kind mismatch unexpectedly succeeded",
                ))
            }
            Err(error) => error,
        };
        assert!(error.contains("kind does not match Runtime-v3"));
        Ok(())
    }

    #[test]
    fn composed_catalog_contains_expert_only_actions_for_durable_dispatch() -> Result<(), String> {
        let expert = expert()?;
        let normal = EpisodeLegalActionSet::new(
            expert.state_id(),
            expert.generation(),
            vec![EpisodeLegalAction::new("end:7", ActionKind::EndTurn)
                .map_err(|error| error.to_string())?],
        )
        .map_err(|error| error.to_string())?;
        let payloads = BTreeMap::from([(
            String::from("end:7"),
            json!({"kind":"end_turn"}),
        )]);
        let (merged, merged_payloads) = merge_actions(&normal, &payloads, &expert)?;
        let catalog = composed_catalog_from_actions(&merged, &merged_payloads)?;
        let actions = catalog.as_array().ok_or("catalog is not an array")?;
        assert!(actions.iter().any(|value| {
            value["action_id"]
                .as_str()
                .is_some_and(|id| id.starts_with("potion:"))
        }));
        Ok(())
    }

    #[test]
    fn expert_settlement_keeps_parameterized_character_and_rest_actions() -> Result<(), String> {
        let mut value = expert()?.as_value().clone();
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
    fn expert_action_request_uses_the_checked_in_action_artifact() -> Result<(), String> {
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
            seed_transport: None,
            gateway_address: String::from("127.0.0.1:15525"),
            gateway_token: String::from("token"),
            mcp_binary: String::from("mcp"),
            runtime_profile: String::from(PROFILE),
            instance_id: String::from("instance-1"),
            caller_id: String::from("harness"),
            session_id: String::from("session-1"),
            lease_id: String::from("lease-1"),
            lease_epoch: 1,
            episode_profile: false,
            mcp_session_id: String::from("mcp-session-1"),
            run_id: String::from("run-1"),
            episode_id: String::from("episode-1"),
            trajectory_id: String::from("trajectory-1"),
            trace_id: String::from("trace-1"),
            artifact_id: String::from("artifact-1"),
            wait_for_combat_seconds: 0,
            settlement_timeout_seconds: 30,
            map_context_enabled: false,
            recovery_environment: Vec::new(),
        };
        let value = action_request(
            &config,
            &identity,
            &action,
            &json!({"kind":"use_potion","potion_id":"potion:fire","target_id":"enemy:1"}),
            "request-1",
        );
        let request = RuntimeV4ExpertActionRequest::from_value(value)
            .map_err(|error| error.to_string())?;
        assert_eq!(request.operation_id(), "episode-action-7-1");
        assert_eq!(request.generation(), 7);
        assert_eq!(request.action_id(), action.action_id());
        Ok(())
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
            seed_transport: None,
            gateway_address: String::from("127.0.0.1:15525"),
            gateway_token: String::from("token"),
            mcp_binary: String::from("mcp"),
            runtime_profile: String::from(PROFILE),
            instance_id: String::from("instance-1"),
            caller_id: String::from("harness"),
            session_id: String::from("session-1"),
            lease_id: String::from("lease-1"),
            lease_epoch: 1,
            episode_profile: false,
            mcp_session_id: String::from("mcp-session-1"),
            run_id: String::from("run-1"),
            episode_id: String::from("episode-1"),
            trajectory_id: String::from("trajectory-1"),
            trace_id: String::from("trace-1"),
            artifact_id: String::from("artifact-1"),
            wait_for_combat_seconds: 0,
            settlement_timeout_seconds: 30,
            map_context_enabled: false,
            recovery_environment: Vec::new(),
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
