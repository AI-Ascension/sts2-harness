// SPDX-License-Identifier: MIT

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sts2_harness::{DecisionInput, ExecutionFingerprint};

use super::super::super::config::RuntimeConfig;
use super::super::super::runtime_v3_settings::RuntimeV3Settings;

const DEFAULT_SEED: &str = "seed:unavailable";
const DEFAULT_BUILD: &str = "build:unavailable";
const DEFAULT_STATE: &str = "state:unavailable";
const MAX_MCP_EXECUTABLE_BYTES: u64 = 128 * 1024 * 1024;

pub(super) fn fingerprint(
    config: &RuntimeConfig,
    settings: &RuntimeV3Settings,
    resume_requested: bool,
) -> Result<ExecutionFingerprint, String> {
    let seed = fingerprint_component(
        "STS2_SEED",
        Some("STS2_VISIBLE_SEED"),
        DEFAULT_SEED,
        resume_requested,
    )?;
    let build = fingerprint_component("STS2_BUILD_DIGEST", None, DEFAULT_BUILD, resume_requested)?;
    let state = fingerprint_component("STS2_STATE_DIGEST", None, DEFAULT_STATE, resume_requested)?;
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
        "mcp_executable": mcp_executable(config.mcp_binary.as_str())?,
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
        .map(sts2_harness::sha256_hex)
        .map_err(|error| format!("cannot hash runtime-v3 evidence: {error}"))
}

pub(super) fn sha256_bytes(bytes: &[u8]) -> String {
    sts2_harness::sha256_hex(bytes)
}

fn digest_text(value: &str) -> String {
    sts2_harness::sha256_hex(value.as_bytes())
}

fn fingerprint_component(
    primary: &str,
    alias: Option<&str>,
    fallback: &str,
    resume_requested: bool,
) -> Result<String, String> {
    let value = optional_env(primary)?.or(match alias {
        Some(alias) => optional_env(alias)?,
        None => None,
    });
    match value {
        Some(value) => Ok(reference_or_digest(&value)),
        None if resume_requested => Err(match alias {
            Some(alias) => {
                format!("runtime-v3 resume requires {primary} or {alias} fingerprint evidence")
            }
            None => format!("runtime-v3 resume requires {primary} fingerprint evidence"),
        }),
        None => Ok(digest_text(fallback)),
    }
}

fn mcp_executable(binary: &str) -> Result<Value, String> {
    let path = resolve_mcp_executable(binary)?;
    let metadata = fs::symlink_metadata(&path)
        .map_err(|error| format!("cannot inspect MCP executable {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "MCP executable {} must not be a symbolic link",
            path.display()
        ));
    }
    if !metadata.file_type().is_file() {
        return Err(format!(
            "MCP executable {} is not a regular file",
            path.display()
        ));
    }
    if metadata.len() > MAX_MCP_EXECUTABLE_BYTES {
        return Err(format!(
            "MCP executable {} exceeds the {}-byte digest bound",
            path.display(),
            MAX_MCP_EXECUTABLE_BYTES
        ));
    }
    let mut file = File::open(&path)
        .map_err(|error| format!("cannot open MCP executable {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 32 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("cannot read MCP executable {}: {error}", path.display()))?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(
                u64::try_from(count).map_err(|_| {
                    format!("MCP executable {} byte count overflowed", path.display())
                })?,
            )
            .ok_or_else(|| format!("MCP executable {} byte count overflowed", path.display()))?;
        if total > MAX_MCP_EXECUTABLE_BYTES {
            return Err(format!(
                "MCP executable {} exceeds the {}-byte digest bound",
                path.display(),
                MAX_MCP_EXECUTABLE_BYTES
            ));
        }
        hasher.update(&buffer[..count]);
    }
    Ok(json!({
        "path": path.to_string_lossy(),
        "sha256": sts2_harness::hex_bytes(hasher.finalize()),
        "bytes": total,
    }))
}

fn resolve_mcp_executable(binary: &str) -> Result<PathBuf, String> {
    let supplied = Path::new(binary);
    if supplied.is_absolute() || supplied.components().count() > 1 {
        return Ok(supplied.to_path_buf());
    }
    let path = std::env::var_os("PATH")
        .ok_or_else(|| format!("cannot resolve MCP executable {binary:?}: PATH is unavailable"))?;
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join(supplied);
        match fs::symlink_metadata(&candidate) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!(
                    "MCP executable {} must not be a symbolic link",
                    candidate.display()
                ));
            }
            Ok(metadata) if metadata.file_type().is_file() => return Ok(candidate),
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!(
                    "cannot inspect MCP executable {}: {error}",
                    candidate.display()
                ));
            }
        }
    }
    Err(format!(
        "cannot resolve MCP executable {binary:?} through PATH"
    ))
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

#[cfg(test)]
#[path = "runtime_v3_durable_support_tests.rs"]
mod tests;
