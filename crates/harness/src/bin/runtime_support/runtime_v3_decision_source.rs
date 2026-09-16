// SPDX-License-Identifier: MIT

fn decision_source(
    config: &RuntimeConfig,
    settings: &RuntimeV3Settings,
    durable: durable::DurableHandle,
    authority_state: lifecycle_authority::RuntimeLifecycleAuthorityState,
    policy_ready: bool,
) -> Result<Box<dyn DecisionSource>, String> {
    if let Some(lookup_agent) = &settings.lookup_agent {
        if !policy_ready {
            return Err(String::from(
                "lookup-agent decision mode requires its selected-policy preflight",
            ));
        }
        Ok(Box::new(
            game_information_decision::LookupAgentDecisionSource::new(
                settings.process.clone(),
                lookup_agent.revision.clone(),
                lookup_agent.timeout,
            ),
        ))
    } else {
        let transport = select_provider_transport(config, settings, durable, authority_state)?;
        let provider = ExoProvider::new(transport, settings.exo.clone());
        Ok(Box::new(ExoDecisionSource::new(ExoSession::new(provider))))
    }
}
