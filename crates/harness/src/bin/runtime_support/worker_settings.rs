// SPDX-License-Identifier: MIT

//! Owner-pinned configuration for the harness worker entry.
//!
//! Worker mode is opt-in.  The normal runtime configuration remains the source of gateway,
//! MCP, episode and trajectory identity; this module only captures the additional launch facts
//! supplied by the owner.  It never reads credentials for the worker transport and never writes
//! process-global environment state.

use std::path::PathBuf;

use sts2_harness::worker_bootstrap::WorkerBootstrap;
use sts2_harness::worker_handoff::{WorkerCommandConfig, WorkerCommandError};
use sts2_harness::{ExecutionFingerprint, WorkerBoot};

use super::config::RuntimeConfig;

const WORKER_OWNER_ID: &str = "harness";
const DEFAULT_STORE_PATH: &str = "harness-execution.sqlite3";

/// Returns whether the executable was explicitly selected as a worker.
pub(super) fn enabled() -> Result<bool, String> {
    match std::env::var("STS2_WORKER_MODE") {
        Ok(value) => parse_enabled(Some(&value)),
        Err(std::env::VarError::NotPresent) => Ok(false),
        Err(std::env::VarError::NotUnicode(_)) => {
            Err(String::from("STS2_WORKER_MODE is not valid UTF-8"))
        }
    }
}

fn parse_enabled(value: Option<&str>) -> Result<bool, String> {
    match value {
        None => Ok(false),
        Some("true") => Ok(true),
        Some("false") => Ok(false),
        Some(_) => Err(String::from(
            "STS2_WORKER_MODE must be exactly true or false",
        )),
    }
}

/// Immutable worker launch material.  None of these values are derived from a worker request.
pub(super) struct WorkerSettings {
    pub(super) boot: WorkerBoot,
    pub(super) command: WorkerCommandConfig,
    pub(super) fingerprint: ExecutionFingerprint,
    pub(super) store_path: PathBuf,
}

impl WorkerSettings {
    pub(super) fn from_environment(
        config: &RuntimeConfig,
        bootstrap: &WorkerBootstrap,
    ) -> Result<Self, String> {
        if config.runtime_profile != "runtime-v3-gameplay" {
            return Err(String::from(
                "worker mode requires the runtime-v3-gameplay profile",
            ));
        }
        if !std::path::Path::new(&config.mcp_binary).is_absolute() {
            return Err(String::from(
                "worker mode requires an absolute approved MCP executable path",
            ));
        }

        let deployment_id = required("STS2_WORKER_DEPLOYMENT_ID")?;
        let worker_profile_digest = required("STS2_WORKER_PROFILE_DIGEST")?;
        let release_digest = required("STS2_BUILD_DIGEST")?;
        let config_digest = required("STS2_RUNTIME_CONFIG_DIGEST")?;
        if std::env::var_os("STS2_WATCHDOG_BOOT_ID").is_some() {
            return Err(String::from(
                "watchdog boot must come from the worker bootstrap pipe",
            ));
        }
        let watchdog_boot_id = bootstrap.watchdog_boot_id().to_owned();
        let worker_owner_id =
            optional("STS2_WORKER_OWNER_ID")?.unwrap_or_else(|| WORKER_OWNER_ID.to_owned());
        if worker_owner_id != WORKER_OWNER_ID {
            return Err(String::from(
                "STS2_WORKER_OWNER_ID must be the stable harness owner",
            ));
        }

        let seed = optional("STS2_SEED")?
            .or(optional("STS2_VISIBLE_SEED")?)
            .ok_or_else(|| String::from("STS2_SEED or STS2_VISIBLE_SEED is required"))?;
        let state_digest = required("STS2_STATE_DIGEST")?;
        let provider_digest = required("STS2_PROVIDER_DIGEST")?;

        let fingerprint = ExecutionFingerprint::new(
            seed,
            release_digest.clone(),
            state_digest,
            config_digest.clone(),
            provider_digest,
        )
        .map_err(|error| format!("worker execution fingerprint is invalid: {error}"))?;
        // The worker boot belongs to this harness process and must not be replayable from a
        // launch environment. The watchdog boot comes exclusively from the dynamic bootstrap
        // frame; native peer authentication remains a separate required boundary.
        let worker_boot_id = fresh_worker_boot_id();
        let command = WorkerCommandConfig::new(
            deployment_id.clone(),
            worker_owner_id.clone(),
            worker_profile_digest.clone(),
            release_digest,
            config_digest,
            worker_boot_id.clone(),
            watchdog_boot_id,
        )
        .map_err(command_error)?;
        let boot = WorkerBoot::new(
            deployment_id,
            worker_owner_id,
            worker_profile_digest,
            worker_boot_id,
        )
        .map_err(|error| format!("worker boot identity is invalid: {error}"))?;
        let store_path = optional("STS2_EXECUTION_STORE_PATH")?
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_STORE_PATH));
        if store_path.as_os_str().is_empty() {
            return Err(String::from("STS2_EXECUTION_STORE_PATH must not be empty"));
        }

        Ok(Self {
            boot,
            command,
            fingerprint,
            store_path,
        })
    }
}

fn command_error(error: WorkerCommandError) -> String {
    format!("worker command binding is invalid: {error}")
}

fn fresh_worker_boot_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn required(name: &str) -> Result<String, String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Ok(value),
        Ok(_) => Err(format!("{name} must not be empty")),
        Err(std::env::VarError::NotPresent) => Err(format!("{name} is required")),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}

fn optional(name: &str) -> Result<Option<String>, String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Ok(Some(value)),
        Ok(_) => Err(format!("{name} must not be empty")),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}

#[cfg(test)]
mod tests {
    use super::{fresh_worker_boot_id, parse_enabled};
    use uuid::Uuid;

    #[test]
    fn worker_mode_requires_an_exact_explicit_boolean() {
        assert_eq!(parse_enabled(None), Ok(false));
        assert_eq!(parse_enabled(Some("true")), Ok(true));
        assert_eq!(parse_enabled(Some("false")), Ok(false));
        for value in ["", "1", "yes", "TRUE", "True", " true", "false "] {
            assert!(
                parse_enabled(Some(value)).is_err(),
                "{value:?} must be rejected"
            );
        }
    }

    #[test]
    fn worker_boot_ids_are_fresh_harness_process_values() {
        let first = fresh_worker_boot_id();
        let second = fresh_worker_boot_id();
        assert_ne!(first, second);
        assert_eq!(
            Uuid::parse_str(&first).map(|id| id.get_version_num()),
            Ok(4)
        );
        assert_eq!(
            Uuid::parse_str(&second).map(|id| id.get_version_num()),
            Ok(4)
        );
    }
}
