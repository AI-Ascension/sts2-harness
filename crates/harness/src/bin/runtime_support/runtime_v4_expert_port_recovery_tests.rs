// SPDX-License-Identifier: MIT

#[cfg(test)]
mod recovery_tests {
    use super::*;

    /// The verified projection with a recovery state, whose condition is carried in `state.code`
    /// and nowhere else.
    fn recovery_expert(code: &str) -> Result<RuntimeV4ExpertObservation, String> {
        let mut value = RuntimeV4ExpertObservation::parse(include_bytes!(
            "../../../../../protocol-artifact/runtime-v4-expert/golden/observation.json"
        ))
        .map_err(|error| error.to_string())?
        .as_value()
        .clone();
        value["state"] = json!({"state": "recovery", "code": code});
        value["legal_actions"] = json!([]);
        RuntimeV4ExpertObservation::from_value(value).map_err(|error| error.to_string())
    }

    /// An expert recovery state is composed through the same rebinding as a playable one, so the
    /// reason is re-bound or the expert profiles report the anonymous recovery the runtime-v3
    /// profile no longer reports.
    #[test]
    fn expert_recovery_observation_carries_the_host_reason() -> Result<(), String> {
        for code in [
            "host_not_configured",
            "launch_contract_refused_isolated_user_dir_mismatch",
        ] {
            let composed = expert_only_observation(&recovery_expert(code)?)?;
            assert_eq!(composed.observation.stage(), EpisodeStage::Recovery);
            assert_eq!(composed.observation.recovery_code(), Some(code));
            assert!(composed.actions.actions().is_empty());
        }
        Ok(())
    }

    /// The recovery reobserve composes the expert projection over the runtime-v3 baseline. The
    /// baseline here deliberately names no reason, so the assertion fails unless the reason comes
    /// out of the expert projection itself.
    #[test]
    fn expert_recovery_composition_carries_the_host_reason() -> Result<(), String> {
        let expert = recovery_expert("host_not_configured")?;
        let baseline = EpisodeObservation::new(
            expert.state_id(),
            expert.generation(),
            EpisodeStage::Recovery,
            false,
            true,
            false,
            expert.as_value().clone(),
        )
        .map_err(|error| error.to_string())?;
        assert_eq!(baseline.recovery_code(), None);
        let actions = EpisodeLegalActionSet::new(expert.state_id(), expert.generation(), Vec::new())
            .map_err(|error| error.to_string())?;
        let composed = compose_with_normal(&baseline, &actions, &BTreeMap::new(), &expert)?;
        assert_eq!(composed.observation.stage(), EpisodeStage::Recovery);
        assert_eq!(
            composed.observation.recovery_code(),
            Some("host_not_configured")
        );
        Ok(())
    }

    /// Only a recovery projection names a condition; a playable one must not acquire a reason.
    #[test]
    fn playable_expert_observation_never_binds_a_recovery_code() -> Result<(), String> {
        let expert = RuntimeV4ExpertObservation::parse(include_bytes!(
            "../../../../../protocol-artifact/runtime-v4-expert/golden/observation.json"
        ))
        .map_err(|error| error.to_string())?;
        let composed = expert_only_observation(&expert)?;
        assert_ne!(composed.observation.stage(), EpisodeStage::Recovery);
        assert_eq!(composed.observation.recovery_code(), None);
        Ok(())
    }
}
