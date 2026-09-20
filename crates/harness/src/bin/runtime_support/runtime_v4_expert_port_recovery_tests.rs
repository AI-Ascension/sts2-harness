// SPDX-License-Identifier: MIT

#[cfg(test)]
mod recovery_carry_tests {
    use super::*;

    const CODE: &str = "launch_contract_refused_host_not_configured";

    fn golden() -> Result<RuntimeV4ExpertObservation, String> {
        RuntimeV4ExpertObservation::parse(include_bytes!(
            "../../../../../protocol-artifact/runtime-v4-expert/golden/observation.json"
        ))
        .map_err(|error| error.to_string())
    }

    /// The host projection of a recovery state names the state but never the runtime-v3 reason.
    fn recovery_expert() -> Result<RuntimeV4ExpertObservation, String> {
        let mut value = golden()?.as_value().clone();
        value["state"] = json!({"state": "recovery", "code": "host_recovery"});
        value["legal_actions"] = Value::Array(Vec::new());
        RuntimeV4ExpertObservation::from_value(value).map_err(|error| error.to_string())
    }

    fn recovery_baseline(
        expert: &RuntimeV4ExpertObservation,
        code: Option<&str>,
    ) -> Result<EpisodeObservation, String> {
        let observation = EpisodeObservation::new(
            expert.state_id(),
            expert.generation(),
            EpisodeStage::Recovery,
            false,
            true,
            false,
            expert.as_value().clone(),
        )
        .map_err(|error| error.to_string())?;
        match code {
            Some(code) => observation
                .with_recovery_code(code)
                .map_err(|error| error.to_string()),
            None => Ok(observation),
        }
    }

    fn empty_catalog(
        expert: &RuntimeV4ExpertObservation,
    ) -> Result<EpisodeLegalActionSet, String> {
        EpisodeLegalActionSet::new(expert.state_id(), expert.generation(), Vec::new())
            .map_err(|error| error.to_string())
    }

    #[test]
    fn composed_normal_recovery_keeps_the_baseline_code() -> Result<(), String> {
        let expert = recovery_expert()?;
        let baseline = recovery_baseline(&expert, Some(CODE))?;
        let composed = compose_with_normal(
            &baseline,
            &empty_catalog(&expert)?,
            &BTreeMap::new(),
            &expert,
        )?;
        assert_eq!(composed.observation.stage(), EpisodeStage::Recovery);
        assert_eq!(composed.observation.recovery_code(), Some(CODE));
        Ok(())
    }

    #[test]
    fn composed_rest_selector_recovery_keeps_the_baseline_code() -> Result<(), String> {
        let expert = recovery_expert()?;
        let baseline = recovery_baseline(&expert, Some(CODE))?;
        let composed =
            compose_with_rest_selector(&baseline, empty_catalog(&expert)?, BTreeMap::new(), &expert)?;
        assert_eq!(composed.observation.stage(), EpisodeStage::Recovery);
        assert_eq!(composed.observation.recovery_code(), Some(CODE));
        Ok(())
    }

    #[test]
    fn rest_selector_overlay_keeps_the_composed_recovery_code() -> Result<(), String> {
        let expert = recovery_expert()?;
        let baseline = recovery_baseline(&expert, Some(CODE))?;
        let mut composed = compose_with_normal(
            &baseline,
            &empty_catalog(&expert)?,
            &BTreeMap::new(),
            &expert,
        )?;
        let selector = json!({
            "legal_actions": [
                {"action_id": "select:7:card:1", "action": {"kind": "select_card", "card_id": "card:1"}}
            ]
        });
        overlay_rest_selector(&mut composed, &selector)?;
        assert_eq!(composed.observation.stage(), EpisodeStage::Recovery);
        assert_eq!(composed.observation.recovery_code(), Some(CODE));
        Ok(())
    }

    /// Composition must not lift a token out of the host projection, which carries none.
    #[test]
    fn composed_recovery_without_a_baseline_code_stays_anonymous() -> Result<(), String> {
        let expert = recovery_expert()?;
        let baseline = recovery_baseline(&expert, None)?;
        let composed = compose_with_normal(
            &baseline,
            &empty_catalog(&expert)?,
            &BTreeMap::new(),
            &expert,
        )?;
        assert_eq!(composed.observation.stage(), EpisodeStage::Recovery);
        assert_eq!(composed.observation.recovery_code(), None);
        Ok(())
    }

    #[test]
    fn a_code_is_refused_outside_a_recovery_stage() -> Result<(), String> {
        let expert = golden()?;
        let combat = EpisodeObservation::new(
            expert.state_id(),
            expert.generation(),
            EpisodeStage::Combat,
            true,
            false,
            true,
            expert.as_value().clone(),
        )
        .map_err(|error| error.to_string())?;
        assert!(carry_recovery_code(combat, Some(CODE)).is_err());
        Ok(())
    }
}
