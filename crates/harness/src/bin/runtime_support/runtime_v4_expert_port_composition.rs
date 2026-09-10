// SPDX-License-Identifier: MIT

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
        "select_player" => ActionKind::SelectPlayer,
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
    fn install_composed(&mut self, composed: &ComposedExpertObservation) -> Result<(), String> {
        self.generation = composed.observation.generation();
        self.current_state = Some(composed.observation.state_id().to_owned());
        self.current_actions = Some(composed.actions.clone());
        let (catalog, catalog_raw) = composed_catalog(&composed.observation)?;
        self.catalog = Some(catalog);
        self.catalog_raw = Some(catalog_raw);
        self.payloads = composed.payloads.clone();
        self.retain_rest_selector();
        if let Some(durable) = &self.durable {
            let catalog_raw = self
                .catalog_raw
                .as_deref()
                .ok_or_else(|| String::from("expert composition has no retained catalog bytes"))?;
            durable.verify_resume_boundary_with_catalog(&composed.observation, catalog_raw)?;
            durable.checkpoint_raw(&composed.observation, catalog_raw)?;
        }
        Ok(())
    }
}
include!("runtime_v4_expert_port_composition_rest.rs");
include!("runtime_v4_expert_port_composition_catalog.rs");
