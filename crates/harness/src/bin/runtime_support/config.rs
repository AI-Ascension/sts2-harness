// SPDX-License-Identifier: MIT

pub(crate) struct RuntimeConfig {
    pub(crate) gateway_address: String,
    pub(crate) gateway_token: String,
    pub(crate) mcp_binary: String,
    pub(crate) instance_id: String,
    pub(crate) caller_id: String,
    pub(crate) session_id: String,
    pub(crate) lease_id: String,
    pub(crate) lease_epoch: u64,
    pub(crate) mcp_session_id: String,
}

impl RuntimeConfig {
    pub(crate) fn from_environment() -> Result<Self, String> {
        let config = Self {
            gateway_address: env_or_default("STS2_GATEWAY_ADDR", "127.0.0.1:15525")?,
            gateway_token: required("STS2_GATEWAY_TOKEN")?,
            mcp_binary: env_or_default("STS2_MCP_BINARY", "sts2-mcp-server")?,
            instance_id: env_or_default("STS2_INSTANCE_ID", "instance-1")?,
            caller_id: env_or_default("STS2_CALLER_ID", "harness")?,
            session_id: env_or_default("STS2_SESSION_ID", "session-1")?,
            lease_id: env_or_default("STS2_LEASE_ID", "lease-1")?,
            lease_epoch: env_or_default("STS2_LEASE_EPOCH", "1")?
                .parse::<u64>()
                .map_err(|_| String::from("STS2_LEASE_EPOCH must be an integer"))?,
            mcp_session_id: env_or_default("STS2_MCP_SESSION_ID", "mcp-session-1")?,
        };
        for (name, value) in [
            ("STS2_INSTANCE_ID", &config.instance_id),
            ("STS2_CALLER_ID", &config.caller_id),
            ("STS2_SESSION_ID", &config.session_id),
            ("STS2_LEASE_ID", &config.lease_id),
            ("STS2_MCP_SESSION_ID", &config.mcp_session_id),
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
        Ok(config)
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
