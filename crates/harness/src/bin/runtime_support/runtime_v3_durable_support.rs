// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sts2_harness::{DecisionInput, ExecutionFingerprint};

use super::super::super::config::RuntimeConfig;
use super::super::super::runtime_v3_settings::RuntimeV3Settings;

const DEFAULT_SEED: &str = "seed:unavailable";
const DEFAULT_BUILD: &str = "build:unavailable";
const DEFAULT_STATE: &str = "state:unavailable";

pub(super) fn fingerprint(
    config: &RuntimeConfig,
    settings: &RuntimeV3Settings,
) -> Result<ExecutionFingerprint, String> {
    let seed = optional_env("STS2_SEED")?
        .or(optional_env("STS2_VISIBLE_SEED")?)
        .map_or_else(
            || digest_text(DEFAULT_SEED),
            |value| reference_or_digest(&value),
        );
    let build = optional_env("STS2_BUILD_DIGEST")?.map_or_else(
        || digest_text(DEFAULT_BUILD),
        |value| reference_or_digest(&value),
    );
    let state = optional_env("STS2_STATE_DIGEST")?.map_or_else(
        || digest_text(DEFAULT_STATE),
        |value| reference_or_digest(&value),
    );
    let config_digest = config_digest(config, settings)?;
    let provider_digest = reference_or_digest(&settings.exo.revision);
    ExecutionFingerprint::new(seed, build, state, config_digest, provider_digest)
        .map_err(|error| format!("runtime-v3 execution fingerprint is invalid: {error}"))
}

pub(super) fn config_digest(
    config: &RuntimeConfig,
    settings: &RuntimeV3Settings,
) -> Result<String, String> {
    let value = json!({
        "runtime_profile": config.runtime_profile,
        "gateway_address": config.gateway_address,
        "mcp_binary": config.mcp_binary,
        "instance_id": config.instance_id,
        "caller_id": config.caller_id,
        "session_id": config.session_id,
        "lease_id": config.lease_id,
        "lease_epoch": config.lease_epoch,
        "mcp_session_id": config.mcp_session_id,
        "run_id": config.run_id,
        "episode_id": config.episode_id,
        "trajectory_id": config.trajectory_id,
        "trace_id": config.trace_id,
        "artifact_id": config.artifact_id,
        "settlement_timeout_seconds": config.settlement_timeout_seconds,
        "exo_revision": settings.exo.revision,
        "exo_max_request_bytes": settings.exo.max_request_bytes,
        "exo_max_response_bytes": settings.exo.max_response_bytes,
        "exo_timeout_millis": settings.exo.timeout_millis,
        "exo_forward_visible_seed": settings.exo.forward_visible_seed,
        "exo_bridge": {
            "executable": settings.process.executable(),
            "arguments": settings.process.arguments(),
            "working_directory": settings.process.working_directory(),
            "inherited_environment": settings.process.inherited_environment(),
        },
        "runner": {
            "max_steps": settings.runner.max_steps(),
            "objective": settings.runner.objective(),
            "hard_constraints": settings.runner.hard_constraints(),
        },
    });
    sha256_json(&value)
}

pub(super) fn decision_input_digest(input: &DecisionInput) -> Result<String, String> {
    let legal_actions: Vec<_> = input
        .legal_actions
        .actions()
        .iter()
        .map(|action| {
            json!({
                "action_id": action.action_id(),
                "kind": super::super::super::runtime_v3_wire::action_kind_name(action.kind()),
            })
        })
        .collect();
    sha256_json(&json!({
        "state_id": input.observation.state_id(),
        "generation": input.observation.generation(),
        "fair_play": input.observation.fair_play().as_value(),
        "legal_actions": legal_actions,
        "objective": input.objective,
        "hard_constraints": input.hard_constraints,
    }))
}

pub(super) fn response_evidence(
    operation_id: &str,
    response: &Value,
) -> Result<(String, String), String> {
    Ok((
        format!("mcp-response-{operation_id}"),
        sha256_json(response)?,
    ))
}

pub(super) fn sha256_json(value: &Value) -> Result<String, String> {
    serde_json::to_vec(value)
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
        .map_err(|error| format!("cannot hash runtime-v3 evidence: {error}"))
}

fn digest_text(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn reference_or_digest(value: &str) -> String {
    if !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control) {
        value.to_owned()
    } else {
        digest_text(value)
    }
}

pub(super) fn optional_env(name: &str) -> Result<Option<String>, String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Ok(Some(value)),
        Ok(_) => Err(format!("{name} must not be empty")),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}
