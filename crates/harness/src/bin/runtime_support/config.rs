// SPDX-License-Identifier: MIT

pub(crate) struct RuntimeConfig {
    pub(crate) gateway_address: String,
    pub(crate) gateway_token: String,
    pub(crate) mcp_binary: String,
    pub(crate) runtime_profile: String,
    pub(crate) instance_id: String,
    pub(crate) caller_id: String,
    pub(crate) session_id: String,
    pub(crate) lease_id: String,
    pub(crate) lease_epoch: u64,
    pub(crate) mcp_session_id: String,
    pub(crate) run_id: String,
    pub(crate) episode_id: String,
    pub(crate) trajectory_id: String,
    pub(crate) artifact_id: String,
    pub(crate) wait_for_combat_seconds: u64,
    pub(crate) settlement_timeout_seconds: u64,
}

impl RuntimeConfig {
    pub(crate) fn from_environment() -> Result<Self, String> {
        let runtime_profile = env_or_default("STS2_RUNTIME_PROFILE", "runtime-v1")?;
        if !matches!(runtime_profile.as_str(), "runtime-v1" | "runtime-v2" | "runtime-v3-gameplay") {
            return Err(String::from(
                "STS2_RUNTIME_PROFILE must be runtime-v1, runtime-v2, or runtime-v3-gameplay",
            ));
        }
        let session_id = env_or_default("STS2_SESSION_ID", "session-1")?;
        let wait_for_combat_seconds = env_or_default("STS2_RUNTIME_WAIT_FOR_COMBAT_SECONDS", "0")?
            .parse::<u64>()
            .map_err(|_| String::from("STS2_RUNTIME_WAIT_FOR_COMBAT_SECONDS must be an integer"))?;
        if wait_for_combat_seconds > 300 {
            return Err(String::from(
                "STS2_RUNTIME_WAIT_FOR_COMBAT_SECONDS must be between 0 and 300",
            ));
        }
        let settlement_timeout_seconds =
            env_or_default("STS2_RUNTIME_SETTLEMENT_TIMEOUT_SECONDS", "30")?
                .parse::<u64>()
                .map_err(|_| {
                    String::from("STS2_RUNTIME_SETTLEMENT_TIMEOUT_SECONDS must be an integer")
                })?;
        if settlement_timeout_seconds > 300 {
            return Err(String::from(
                "STS2_RUNTIME_SETTLEMENT_TIMEOUT_SECONDS must be between 0 and 300",
            ));
        }
        let config = Self {
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
            mcp_session_id: env_or_default("STS2_MCP_SESSION_ID", "mcp-session-1")?,
            run_id: env_or_default("STS2_RUN_ID", "run-runtime-0001")?,
            episode_id: env_or_default("STS2_EPISODE_ID", "episode-runtime-0001")?,
            trajectory_id: env_or_default("STS2_TRAJECTORY_ID", "trajectory-runtime-0001")?,
            artifact_id: env_or_default("STS2_ARTIFACT_ID", "artifact-runtime-0001")?,
            wait_for_combat_seconds,
            settlement_timeout_seconds,
        };
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), String> {
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
            &config.artifact_id,
        ];
        for (index, value) in lineage_ids.iter().enumerate() {
            if lineage_ids[..index].contains(value) {
                return Err(String::from(
                    "STS2 run, episode, trajectory, and artifact identities must be distinct",
                ));
            }
        }
        Ok(())
    }
}

fn required(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is required"))
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

#[cfg(test)]
mod tests {
    use super::RuntimeConfig;

    #[test]
    fn runtime_sessions_are_validated_independently() {
        let mut config = RuntimeConfig {
            gateway_address: String::from("127.0.0.1:15525"),
            gateway_token: String::from("synthetic-token"),
            mcp_binary: String::from("mcp"),
            runtime_profile: String::from("runtime-v3-gameplay"),
            instance_id: String::from("instance-1"),
            caller_id: String::from("harness"),
            session_id: String::from("gateway-session-1"),
            lease_id: String::from("lease-1"),
            lease_epoch: 1,
            mcp_session_id: String::from("mcp-session-independent"),
        };
        assert!(config.validate().is_ok());
        config.mcp_session_id = String::from("unsafe session");
        assert!(config.validate().is_err());
        config.mcp_session_id = String::from("mcp-session-independent");
        config.session_id.clear();
        assert!(config.validate().is_err());
    }
}
