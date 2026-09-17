// SPDX-License-Identifier: MIT

#[derive(Clone)]
pub(crate) struct RuntimeConfig {
    #[allow(dead_code)]
    pub(crate) seed_transport: Option<super::seed_transport::SeedTransportConfig>,
    pub(crate) gateway_address: String,
    pub(crate) gateway_token: String,
    pub(crate) mcp_binary: String,
    pub(crate) runtime_profile: String,
    pub(crate) instance_id: String,
    pub(crate) caller_id: String,
    pub(crate) session_id: String,
    pub(crate) lease_id: String,
    pub(crate) lease_epoch: u64,
    /// Opt-in negotiation of the gateway's repeated-episode lease profile
    /// (`sts2-gateway#67`). Off by default so an ordinary run keeps the
    /// gateway's permanent-revocation single-episode contract.
    pub(crate) episode_profile: bool,
    pub(crate) mcp_session_id: String,
    pub(crate) run_id: String,
    pub(crate) episode_id: String,
    pub(crate) trajectory_id: String,
    pub(crate) trace_id: String,
    pub(crate) artifact_id: String,
    pub(crate) wait_for_combat_seconds: u64,
    pub(crate) settlement_timeout_seconds: u64,
    pub(crate) map_context_enabled: bool,
    /// Recovery credentials and identity are captured before child environments are scrubbed.
    pub(crate) recovery_environment: Vec<(String, String)>,
}

impl RuntimeConfig {
    /// Loads the explicitly selected durable branch, if both selector variables are present.
    ///
    /// Experiment and branch identities are separate namespaces. A partial selector is rejected
    /// so a runtime can never guess which experiment owns a branch.
    pub(crate) fn branch_continuation_selector()
    -> Result<Option<sts2_harness::BranchContinuationSelector>, String> {
        let experiment_id = optional_identity("STS2_EXPERIMENT_ID")?;
        let branch_id = optional_identity("STS2_BRANCH_ID")?;
        Self::selector_from_values(experiment_id, branch_id)
    }

    fn selector_from_values(
        experiment_id: Option<String>,
        branch_id: Option<String>,
    ) -> Result<Option<sts2_harness::BranchContinuationSelector>, String> {
        match (experiment_id, branch_id) {
            (None, None) => Ok(None),
            (Some(experiment_id), Some(branch_id)) => {
                sts2_harness::BranchContinuationSelector::new(experiment_id, branch_id)
                    .map(Some)
                    .map_err(|_| {
                        String::from(
                            "STS2_EXPERIMENT_ID or STS2_BRANCH_ID is empty, unsafe, or oversized",
                        )
                    })
            }
            _ => Err(String::from(
                "STS2_EXPERIMENT_ID and STS2_BRANCH_ID must be set together",
            )),
        }
    }

    pub(crate) fn recovery_value(&self, name: &str) -> Option<&str> {
        self.recovery_environment
            .iter()
            .find_map(|(key, value)| (key == name).then_some(value.as_str()))
    }

    pub(crate) fn from_environment() -> Result<Self, String> {
        let runtime_profile = env_or_default("STS2_RUNTIME_PROFILE", "runtime-v1")?;
        if !matches!(
            runtime_profile.as_str(),
            "runtime-v1"
                | "runtime-v2"
                | "runtime-v3-gameplay"
                | "negotiated-composition-v1"
                | "runtime-v4-expert"
                | "runtime-v4-expert-rest-action"
        ) {
            return Err(String::from(
                "STS2_RUNTIME_PROFILE must be runtime-v1, runtime-v2, runtime-v3-gameplay, negotiated-composition-v1, runtime-v4-expert, or runtime-v4-expert-rest-action",
            ));
        }
        let session_id = env_or_default("STS2_SESSION_ID", "session-1")?;
        let seed_transport = super::seed_transport::SeedTransportConfig::from_environment()?;
        let wait_for_combat_seconds = bounded_seconds("STS2_RUNTIME_WAIT_FOR_COMBAT_SECONDS", "0")?;
        let settlement_timeout_seconds =
            bounded_seconds("STS2_RUNTIME_SETTLEMENT_TIMEOUT_SECONDS", "30")?;
        let map_context_enabled = flag_with_default("STS2_ENABLE_MAP_CONTEXT", false)?;
        let config = Self {
            seed_transport,
            gateway_address: env_or_default("STS2_GATEWAY_ADDR", "127.0.0.1:15525")?,
            gateway_token: required("STS2_GATEWAY_TOKEN")?,
            mcp_binary: env_or_default("STS2_MCP_BINARY", "sts2-mcp-server")?,
            runtime_profile,
            instance_id: env_or_default("STS2_INSTANCE_ID", "instance-1")?,
            caller_id: env_or_default("STS2_CALLER_ID", "harness")?,
            session_id: session_id.clone(),
            lease_id: env_or_default("STS2_LEASE_ID", "lease-1")?,
            lease_epoch: env_or_default("STS2_LEASE_EPOCH", "1")?
                .parse::<u64>()
                .map_err(|_| String::from("STS2_LEASE_EPOCH must be an integer"))?,
            episode_profile: optional_flag("STS2_EPISODE_PROFILE")?,
            mcp_session_id: env_or_default("STS2_MCP_SESSION_ID", "mcp-session-1")?,
            run_id: env_or_default("STS2_RUN_ID", "run-runtime-0001")?,
            episode_id: env_or_default("STS2_EPISODE_ID", "episode-runtime-0001")?,
            trajectory_id: env_or_default("STS2_TRAJECTORY_ID", "trajectory-runtime-0001")?,
            trace_id: env_or_default("STS2_TRACE_ID", "trace-runtime-0001")?,
            artifact_id: env_or_default("STS2_ARTIFACT_ID", "artifact-runtime-0001")?,
            wait_for_combat_seconds,
            settlement_timeout_seconds,
            map_context_enabled,
            recovery_environment: [
                "STS2_RECOVERY_TOKEN",
                "STS2_RECOVERY_PRINCIPAL_ID",
                "STS2_RECOVERY_ROLE",
                "STS2_RECOVERY_PROOF",
                "STS2_RECOVERY_DEPLOYMENT_ID",
                "STS2_RECOVERY_INSTANCE_ID",
                "STS2_RECOVERY_INSTANCE_INCAR",
                "STS2_RECOVERY_BOOT_ID",
                "STS2_RECOVERY_LEASE_ID",
                "STS2_RECOVERY_AUTHORITY_GENERATION",
                "STS2_RECOVERY_LEASE_EPOCH",
                "STS2_RECOVERY_CURRENT_FENCE_JSON",
            ]
            .into_iter()
            .filter_map(|name| {
                std::env::var(name)
                    .ok()
                    .map(|value| (name.to_owned(), value))
            })
            .collect(),
        };
        config.validate()?;
        Self::branch_continuation_selector()?;
        Ok(config)
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        let config = self;
        for (name, value) in [
            ("STS2_INSTANCE_ID", &config.instance_id),
            ("STS2_CALLER_ID", &config.caller_id),
            ("STS2_SESSION_ID", &config.session_id),
            ("STS2_LEASE_ID", &config.lease_id),
            ("STS2_MCP_SESSION_ID", &config.mcp_session_id),
            ("STS2_RUN_ID", &config.run_id),
            ("STS2_EPISODE_ID", &config.episode_id),
            ("STS2_TRAJECTORY_ID", &config.trajectory_id),
            ("STS2_TRACE_ID", &config.trace_id),
            ("STS2_ARTIFACT_ID", &config.artifact_id),
        ] {
            if !safe_identity(value) {
                return Err(format!("{name} is empty, unsafe, or oversized"));
            }
        }
        if config.gateway_token.is_empty()
            || config.gateway_token.len() > 256
            || config
                .gateway_token
                .bytes()
                .any(|byte| byte.is_ascii_whitespace())
        {
            return Err(String::from(
                "STS2_GATEWAY_TOKEN is empty, unsafe, or oversized",
            ));
        }
        if config.session_id == config.mcp_session_id {
            return Err(String::from(
                "STS2_SESSION_ID and STS2_MCP_SESSION_ID must be distinct",
            ));
        }
        let lineage_ids = [
            &config.run_id,
            &config.episode_id,
            &config.trajectory_id,
            &config.trace_id,
            &config.artifact_id,
        ];
        for (index, value) in lineage_ids.iter().enumerate() {
            if lineage_ids[..index].contains(value) {
                return Err(String::from(
                    "STS2 run, episode, trajectory, trace, and artifact identities must be distinct",
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn lookup_binding_enabled(&self) -> Result<bool, String> {
        flag_with_default("STS2_ENABLE_GAME_INFORMATION_LOOKUP_BINDING", false)
    }

    pub(crate) fn lookup_scope(&self) -> Result<(String, String, u64), String> {
        let (project_id, agent_id) = self.lookup_scope_identity()?;
        let authority_epoch = env_or_default("STS2_AUTHORITY_EPOCH", "1")?
            .parse::<u64>()
            .map_err(|_| String::from("STS2_AUTHORITY_EPOCH must be an integer"))?;
        Ok((project_id, agent_id, authority_epoch))
    }

    pub(crate) fn lookup_scope_identity(&self) -> Result<(String, String), String> {
        let project_id = env_or_default("STS2_PROJECT_ID", "project-runtime-0001")?;
        let agent_id = env_or_default("STS2_AGENT_ID", "agent-runtime-0001")?;
        if !safe_identity(&project_id) || !safe_identity(&agent_id) {
            return Err(String::from(
                "STS2_PROJECT_ID or STS2_AGENT_ID is empty, unsafe, or oversized",
            ));
        }
        Ok((project_id, agent_id))
    }
}

fn required(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is required"))
}

fn optional_identity(name: &str) -> Result<Option<String>, String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Ok(Some(value)),
        Ok(_) => Err(format!("{name} must not be empty")),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}

/// Opt-in boolean flag. Absent means `false`; a present-but-non-boolean value is
/// rejected rather than guessed, so a typo cannot silently disable a fence.
fn optional_flag(name: &str) -> Result<bool, String> {
    match std::env::var(name) {
        Ok(value) if value.is_empty() => Err(format!("{name} must not be empty")),
        Ok(value) => value
            .parse::<bool>()
            .map_err(|_| format!("{name} must be true or false")),
        Err(std::env::VarError::NotPresent) => Ok(false),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}

fn bounded_seconds(name: &str, default: &str) -> Result<u64, String> {
    let seconds = env_or_default(name, default)?
        .parse::<u64>()
        .map_err(|_| format!("{name} must be an integer"))?;
    if seconds > 300 {
        return Err(format!("{name} must be between 0 and 300"));
    }
    Ok(seconds)
}

fn env_or_default(name: &str, default: &str) -> Result<String, String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Ok(value),
        Ok(_) => Err(format!("{name} must not be empty")),
        Err(std::env::VarError::NotPresent) => Ok(String::from(default)),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}

fn safe_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.contains("..")
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
}

fn flag_with_default(name: &str, default: bool) -> Result<bool, String> {
    match std::env::var(name) {
        Ok(value) => parse_flag(name, &value),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}

fn parse_flag(name: &str, value: &str) -> Result<bool, String> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("{name} must be exactly true or false")),
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
