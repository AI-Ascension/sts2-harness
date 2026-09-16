// SPDX-License-Identifier: MIT

fn exo_identity(_config: &RuntimeConfig, settings: &RuntimeV3Settings) -> Result<Value, String> {
    if let Some(lookup_agent) = &settings.lookup_agent {
        let inherited_environment = settings
            .process
            .inherited_environment()
            .iter()
            .map(|name| {
                let value = std::env::var(name)
                    .ok()
                    .map(|value| sha256_bytes(value.as_bytes()));
                (name.clone(), json!(value))
            })
            .collect::<serde_json::Map<_, _>>();
        return Ok(json!({
            "contract_version": "ascension.game-information-query-v1",
            "decision_source": "lookup_agent",
            "binary_sha256": lookup_agent.revision,
            "owner_config_sha256": lookup_agent.owner_config_sha256,
            "timeout_millis": lookup_agent.timeout.as_millis(),
            "process": {
                "executable": settings.process.executable(),
                "arguments": settings.process.arguments(),
                "working_directory": settings.process.working_directory(),
                "inherited_environment_names": settings.process.inherited_environment(),
                "inherited_environment_value_sha256": inherited_environment,
            },
        }));
    }
    Ok(json!({
        "contract_version": EXO_CONTRACT_VERSION,
        "source_revision": settings.exo.revision,
        "package_digest": identity_axis("STS2_EXO_PACKAGE_DIGEST")?,
        "extension_digest": identity_axis("STS2_EXO_EXTENSION_DIGEST")?,
        "bridge_digest": identity_axis("STS2_EXO_BRIDGE_DIGEST")?,
        "model_binding": identity_axis("STS2_EXO_MODEL_BINDING")?,
        "prompt_digest": identity_axis("STS2_EXO_PROMPT_DIGEST")?,
        "tool_digest": identity_axis("STS2_EXO_TOOL_DIGEST")?,
        "config_digest": identity_axis("STS2_EXO_CONFIG_DIGEST")?,
        "native_instance_id": identity_axis("STS2_EXO_NATIVE_INSTANCE_ID")?,
    }))
}
