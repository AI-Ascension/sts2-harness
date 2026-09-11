// SPDX-License-Identifier: MIT

impl RuntimeV3Port {
    fn validate_rest_selection_completion(
        &self,
        result: &RuntimeV4ExpertRestActionResult,
        action: &EpisodeLegalAction,
        selector_context: Option<&Value>,
    ) -> Result<(), String> {
        let Some(transition) = result.transition() else {
            return Ok(());
        };
        if transition["kind"] != "rest_option_selection_completed" {
            return Ok(());
        }
        let selector = selector_context
            .ok_or_else(|| String::from("REST selection completion has no retained selector"))?;
        if selector["selection_id"] != transition["selection_id"]
            || selector["selection_kind"] != transition["selection_kind"]
            || selector["required_count"] != transition["required_count"]
        {
            return Err(String::from(
                "REST selection completion does not match the retained selector",
            ));
        }
        let action_reference = selector["legal_actions"]
            .as_array()
            .and_then(|actions| {
                actions.iter().find(|reference| {
                    reference["action_id"].as_str() == Some(action.action_id())
                })
            })
            .ok_or_else(|| {
                String::from("REST selection completion action is absent from the retained selector")
            })?;
        if action_reference["action"] != result.as_value()["action"]["action"] {
            return Err(String::from(
                "REST selection completion action differs from the retained selector",
            ));
        }
        let selected = transition["selected_choice_ids"]
            .as_array()
            .ok_or_else(|| String::from("REST selection completion omitted selected choices"))?;
        let mut catalog_choices = std::collections::BTreeSet::new();
        for choice in selector["selected_choice_ids"]
            .as_array()
            .ok_or_else(|| String::from("REST retained selector omitted selected choices"))?
        {
            let choice_id = choice
                .as_str()
                .ok_or_else(|| String::from("REST retained selector has an invalid choice"))?;
            catalog_choices.insert(choice_id);
        }
        for reference in selector["legal_actions"]
            .as_array()
            .ok_or_else(|| String::from("REST retained selector omitted legal actions"))?
        {
            let payload = &reference["action"];
            let choice_id = match payload["kind"].as_str() {
                Some("select_card") => payload["card_id"].as_str(),
                Some("select_player") => payload["player_id"].as_str(),
                _ => None,
            };
            if let Some(choice_id) = choice_id {
                catalog_choices.insert(choice_id);
            }
        }
        if selected.iter().any(|choice| {
            choice
                .as_str()
                .is_none_or(|choice_id| !catalog_choices.contains(choice_id))
        }) {
            return Err(String::from(
                "REST selection completion contains a choice absent from the retained catalog",
            ));
        }
        if action.kind() == ActionKind::SelectPlayer {
            let target = result.as_value()["action"]["action"]["player_id"]
                .as_str()
                .ok_or_else(|| String::from("REST player completion target is unavailable"))?;
            if !catalog_choices.contains(target) {
                return Err(String::from(
                    "REST player completion target is absent from the retained catalog",
                ));
            }
        }
        Ok(())
    }
}
