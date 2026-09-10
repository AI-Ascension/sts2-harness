// SPDX-License-Identifier: MIT

impl RuntimeV3Port {
    fn dispatch_rest_action(
        &mut self,
        identity: &ActionIdentity,
        action: &EpisodeLegalAction,
        payload: Value,
    ) -> Result<TransitionReceipt, sts2_harness::PortError> {
        let request = RuntimeV4ExpertRestActionRequest::from_value(rest_action_request(
            &self.config,
            identity,
            action,
            &payload,
        ))
        .map_err(|error| wire::port_error("rest_action_request_invalid", error.to_string(), false))?;
        let payload_digest = wire::canonical_action_digest(action.action_id(), &payload)
            .map_err(|error| wire::port_error("dispatch_intent_failed", error, false))?;
        let (_, value) = self
            .call_expert_tool(
                REST_ACTION_TOOL,
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
            .map_err(|error| wire::port_error("rest_action_dispatch_failed", error, true))?;
        let result = RuntimeV4ExpertRestActionResult::from_value(value).map_err(|error| {
            wire::port_error("rest_action_dispatch_invalid", error.to_string(), false)
        })?;
        let response = result.as_value().clone();
        let selector_context = self.rest_selector_value.clone();
        let receipt = self
            .rest_result_receipt(
                result,
                &request,
                identity,
                action,
                selector_context.as_ref(),
            )
            .map_err(|error| wire::port_error("rest_action_receipt_invalid", error, false))?;
        self.record_durable_receipt(
            &identity.operation_id,
            &payload_digest,
            &receipt,
            &response,
        )
        .map_err(|error| wire::port_error("dispatch_durability_failed", error, false))?;
        super::recording::receipt(&receipt, identity.generation, &self.telemetry);
        Ok(receipt)
    }

    fn reconcile_rest_operation(&mut self, operation_id: &str) -> Result<TransitionReceipt, String> {
        let record = self
            .operations
            .get(operation_id)
            .cloned()
            .ok_or_else(|| String::from("REST action is not in the operation ledger"))?;
        // The legacy MCP recovery path must rehydrate the caller's bounded identity exactly;
        // the UUIDv4 requirement is enforced at production action creation and by the historical
        // recovery sideband, not by compatibility callers using the ordinary recover tool.
        let identity = ActionIdentity::new(
            operation_id.to_owned(),
            record.state_id.clone(),
            record.generation,
            record.action.action_id().to_owned(),
        )
        .map_err(|error| error.to_string())?;
        let request = RuntimeV4ExpertRestActionRequest::from_value(rest_action_request(
            &self.config,
            &identity,
            &record.action,
            &record.payload,
        ))
        .map_err(|error| error.to_string())?;
        let (_, value) = self.call_expert_tool(
            REST_RECONCILE_TOOL,
            json!({
                "instance_id": self.config.instance_id,
                "mcp_session_id": self.config.mcp_session_id,
                "lease_id": self.config.lease_id,
                "lease_epoch": self.config.lease_epoch,
                "operation_id": operation_id
            }),
        )?;
        let result = RuntimeV4ExpertRestActionResult::from_value(value)
            .map_err(|error| format!("REST action reconcile response is invalid: {error}"))?;
        if matches!(
            result.status(),
            RuntimeV4ExpertRestActionStatus::Settled
                | RuntimeV4ExpertRestActionStatus::Rejected
                | RuntimeV4ExpertRestActionStatus::Cancelled
        ) && let Some(durable) = &self.durable
        {
            durable.clear_resume_boundary()?;
        }
        let response = result.as_value().clone();
        let receipt = self.rest_result_receipt(
            result,
            &request,
            &identity,
            &record.action,
            record.rest_selector.as_ref(),
        )?;
        let payload_digest = self
            .durable
            .as_ref()
            .map_or_else(
                || Ok(String::new()),
                |durable| durable.operation_payload_digest(operation_id),
            )?;
        self.record_reconciled_durable_receipt(operation_id, &payload_digest, &receipt, &response)?;
        Ok(receipt)
    }

    fn rest_result_receipt(
        &mut self,
        result: RuntimeV4ExpertRestActionResult,
        request: &RuntimeV4ExpertRestActionRequest,
        identity: &ActionIdentity,
        action: &EpisodeLegalAction,
        selector_context: Option<&Value>,
    ) -> Result<TransitionReceipt, String> {
        result
            .matches_request(request)
            .map_err(|error| format!("REST action response identity mismatch: {error}"))?;
        if result.as_value()["action"]
            .get("action_id")
            .and_then(Value::as_str)
            != Some(action.action_id())
        {
            return Err(String::from("REST action response changed the action identity"));
        }
        self.validate_rest_selection_completion(&result, action, selector_context)?;
        let status = match result.status() {
            RuntimeV4ExpertRestActionStatus::Accepted => DispatchStatus::Accepted,
            RuntimeV4ExpertRestActionStatus::Settled => DispatchStatus::Settled,
            RuntimeV4ExpertRestActionStatus::Rejected => DispatchStatus::Rejected,
            RuntimeV4ExpertRestActionStatus::Unknown => DispatchStatus::Unknown,
            RuntimeV4ExpertRestActionStatus::Cancelled => DispatchStatus::Cancelled,
        };
        let after = if status == DispatchStatus::Settled {
            let observation = result
                .observation()
                .cloned()
                .ok_or_else(|| String::from("REST settlement omitted observation"))?;
            let expert = RuntimeV4ExpertObservation::from_value(observation)
                .map_err(|error| format!("REST settlement observation is invalid: {error}"))?;
            if expert.state_id() != result.as_value()["state_id"].as_str().unwrap_or("")
                || expert.generation() != result.generation()
            {
                return Err(String::from("REST settlement observation identity is inconsistent"));
            }
            let mut composed = expert_only_observation(&expert)?;
            if let Some(transition) = result.transition()
                && matches!(
                    transition["kind"].as_str(),
                    Some("rest_option_selection_requested" | "rest_option_selection_progressed")
                )
            {
                self.install_rest_selector(
                    &transition["selector"],
                    expert.state_id(),
                    expert.generation(),
                )?;
                self.apply_active_rest_selector(&mut composed)?;
            }
            self.install_composed(&composed)?;
            Some(composed.observation)
        } else {
            None
        };
        let effect_kind = result
            .transition_kind()
            .map(str::to_owned)
            .or_else(|| (status == DispatchStatus::Settled).then(|| String::from("rest_action_settled")));
        Ok(TransitionReceipt::new(
            identity.operation_id.clone(),
            action.clone(),
            status,
            after,
            effect_kind,
            result.as_value()["error_code"].as_str().map(str::to_owned),
        ))
    }

    fn install_rest_selector(
        &mut self,
        selector: &Value,
        state_id: &str,
        generation: u64,
    ) -> Result<(), String> {
        let values = selector
            .get("legal_actions")
            .and_then(Value::as_array)
            .ok_or_else(|| String::from("REST selector omitted legal actions"))?;
        let mut actions = Vec::with_capacity(values.len());
        let mut payloads = std::collections::BTreeMap::new();
        for value in values {
            let action_id = value["action_id"]
                .as_str()
                .ok_or_else(|| String::from("REST selector action ID is invalid"))?;
            let payload = value
                .get("action")
                .cloned()
                .ok_or_else(|| String::from("REST selector action payload is missing"))?;
            let kind = payload["kind"]
                .as_str()
                .and_then(expert_action_kind)
                .ok_or_else(|| String::from("REST selector action kind is unsupported"))?;
            let action = EpisodeLegalAction::new(action_id, kind).map_err(|error| error.to_string())?;
            payloads.insert(action_id.to_owned(), payload);
            actions.push(action);
        }
        let action_set = EpisodeLegalActionSet::new(state_id, generation, actions)
            .map_err(|error| error.to_string())?;
        self.rest_selector_actions = Some(action_set);
        self.rest_selector_payloads = payloads;
        self.rest_selector_value = Some(selector.clone());
        Ok(())
    }
}

include!("runtime_v4_expert_port_rest_overlay.rs");

fn rest_action_request(
    config: &super::RuntimeConfig,
    identity: &ActionIdentity,
    action: &EpisodeLegalAction,
    payload: &Value,
) -> Value {
    json!({
        "protocol_version": sts2_harness::RUNTIME_V4_EXPERT_REST_ACTION_PROTOCOL_VERSION,
        "schema_digest": sts2_harness::RUNTIME_V4_EXPERT_REST_ACTION_SCHEMA_DIGEST,
        "provenance": {
            "artifact": sts2_harness::RUNTIME_V4_EXPERT_REST_ACTION_ARTIFACT,
            "source": sts2_harness::RUNTIME_V4_EXPERT_REST_ACTION_SCHEMA_SOURCE,
            "generator": sts2_harness::RUNTIME_V4_EXPERT_REST_ACTION_GENERATOR
        },
        "profile": "expert-rest-action",
        "correlation_id": "request",
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
        "effect_witness": null,
        "error_code": null
    })
}
