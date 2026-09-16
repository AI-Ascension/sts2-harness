// SPDX-License-Identifier: MIT

fn read_runtime_legal_actions(
    port: &mut RuntimeV3Port,
    state_id: &str,
    generation: u64,
) -> Result<sts2_harness::EpisodeLegalActionSet, sts2_harness::PortError> {
        let mut arguments = port.context(generation);
        if let Value::Object(object) = &mut arguments {
            object.insert(String::from("state_id"), Value::String(state_id.to_owned()));
        }
        let value = match port.call_tool_classified("sts2.legal_actions", arguments) {
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
        let response_text = port.last_response_text.clone().ok_or_else(|| {
            wire::port_error("legal_actions_invalid", "MCP response text missing", false)
        })?;
        let parsed = parse::action_set_with_catalog_text(
            &value,
            &response_text,
            "legal_actions_response",
            &port.config,
        )
        .map_err(|error| wire::port_error("legal_actions_invalid", error, false))?;
        let actions = parsed.actions;
        let payloads = parsed.payloads;
        port.catalog = Some(parsed.catalog);
        port.catalog_raw = Some(parsed.catalog_raw);
        port.generation = actions.generation();
        port.current_state = Some(actions.state_id().to_owned());
        port.current_actions = Some(actions.clone());
        port.payloads = payloads;
        let actions = if port.is_expert_profile() {
            let actions = port.expert_catalog(state_id, generation)?;
            if port.is_rest_profile()
                && port
                    .rest_selector_actions
                    .as_ref()
                    .is_some_and(|selector| selector.assert_matches(state_id, generation).is_ok())
            {
                let selector = port.rest_selector_actions.clone().ok_or_else(|| {
                    wire::port_error("rest_selector_invalid", "selector disappeared", false)
                })?;
                port.current_actions = Some(selector.clone());
                port.payloads = port.rest_selector_payloads.clone();
                let (catalog, catalog_raw) =
                    super::expert::composed_catalog(&selector, &port.rest_selector_payloads)
                        .map_err(|error| {
                            wire::port_error("expert_legal_actions_invalid", error, false)
                        })?;
                port.catalog = Some(catalog);
                port.catalog_raw = Some(catalog_raw);
                selector
            } else {
                let (catalog, catalog_raw) =
                    super::expert::composed_catalog(&actions, &port.payloads)
                        .map_err(|error| {
                            wire::port_error("expert_legal_actions_invalid", error, false)
                        })?;
                port.catalog = Some(catalog);
                port.catalog_raw = Some(catalog_raw);
                actions
            }
        } else {
            actions
        };
        port.update_lifecycle_catalog_authority(&actions)?;
        Ok(actions)
}
