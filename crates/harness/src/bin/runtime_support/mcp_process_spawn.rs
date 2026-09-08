// SPDX-License-Identifier: MIT

use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use tokio::io::BufReader;
use tokio::process::Command;

use super::super::config::RuntimeConfig;
use super::McpProcess;

const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(5);

impl McpProcess {
    pub(in super::super) fn spawn(config: &RuntimeConfig) -> Result<Self, String> {
        Self::spawn_command(Self::configured_command(config), EXCHANGE_TIMEOUT)
    }

    pub(in super::super) fn spawn_with_cancellation(
        config: &RuntimeConfig,
        cancellation: &sts2_harness::ExecutionCancellation,
    ) -> Result<Self, String> {
        if cancellation.is_cancelled() {
            return Err(String::from("MCP launch cancelled"));
        }
        let mut process = Self::spawn(config)?;
        process.cancellation = cancellation.clone();
        Ok(process)
    }

    pub(in super::super) fn spawn_recovery(
        config: &RuntimeConfig,
        instance_id: &str,
        lease_id: &str,
        lease_epoch: u64,
        authority: &Value,
        cancellation: &sts2_harness::ExecutionCancellation,
    ) -> Result<Self, String> {
        if cancellation.is_cancelled() {
            return Err(String::from("MCP recovery launch cancelled"));
        }
        let mut command = Self::configured_command(config);
        command
            .env("STS2_RUNTIME_PROFILE", "watchdog-recovery-v1")
            .env("STS2_INSTANCE_ID", instance_id)
            .env("STS2_LEASE_ID", lease_id)
            .env("STS2_LEASE_EPOCH", lease_epoch.to_string());
        for name in [
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
        ] {
            if let Some(value) = config.recovery_value(name) {
                command.env(name, value);
            }
        }
        for (name, value) in [
            (
                "STS2_RECOVERY_DEPLOYMENT_ID",
                authority["deployment_id"].as_str(),
            ),
            (
                "STS2_RECOVERY_INSTANCE_ID",
                authority["instance_id"].as_str(),
            ),
            (
                "STS2_RECOVERY_INSTANCE_INCAR",
                authority["instance_incarnation"].as_str(),
            ),
            ("STS2_RECOVERY_BOOT_ID", authority["boot_id"].as_str()),
            ("STS2_RECOVERY_LEASE_ID", authority["lease_id"].as_str()),
        ] {
            let value = value.ok_or("validated recovery authority omitted an identity")?;
            command.env(name, value);
        }
        let authority_generation = authority["authority_generation"]
            .as_u64()
            .ok_or("validated recovery authority omitted authority_generation")?;
        let lease_epoch = authority["lease_epoch"]
            .as_u64()
            .ok_or("validated recovery authority omitted lease_epoch")?;
        let current_fence = serde_json::to_string(&authority["current_fence"])
            .map_err(|_| "validated recovery authority fence could not be encoded")?;
        command
            .env(
                "STS2_RECOVERY_AUTHORITY_GENERATION",
                authority_generation.to_string(),
            )
            .env("STS2_RECOVERY_LEASE_EPOCH", lease_epoch.to_string())
            .env("STS2_RECOVERY_CURRENT_FENCE_JSON", current_fence);
        if config.recovery_value("STS2_RECOVERY_TOKEN").is_none() {
            return Err(String::from(
                "STS2_RECOVERY_TOKEN is required for the recovery sideband",
            ));
        }
        if cancellation.is_cancelled() {
            return Err(String::from("MCP recovery launch cancelled"));
        }
        let mut process = Self::spawn_command(command, EXCHANGE_TIMEOUT)?;
        process.cancellation = cancellation.clone();
        Ok(process)
    }

    pub(in super::super) fn configured_command(config: &RuntimeConfig) -> Command {
        let mut command = Command::new(&config.mcp_binary);
        command.env_clear();
        for name in ["PATH", "SystemRoot", "TEMP", "TMP"] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command
            .env("STS2_GATEWAY_ADDR", &config.gateway_address)
            .env("STS2_GATEWAY_TOKEN", &config.gateway_token)
            .env("STS2_RUNTIME_PROFILE", &config.runtime_profile)
            .env("STS2_INSTANCE_ID", &config.instance_id)
            .env("STS2_CALLER_ID", &config.caller_id)
            .env("STS2_SESSION_ID", &config.session_id)
            .env("STS2_MCP_SESSION_ID", &config.mcp_session_id)
            .env("STS2_LEASE_ID", &config.lease_id)
            .env("STS2_LEASE_EPOCH", config.lease_epoch.to_string());
        command
    }

    pub(in super::super) fn spawn_command(
        mut command: Command,
        timeout: Duration,
    ) -> Result<Self, String> {
        super::supervised(|| Self::spawn_supervised(&mut command, timeout))
    }

    fn spawn_supervised(command: &mut Command, timeout: Duration) -> Result<Self, &'static str> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| "MCP supervisor unavailable")?;
        let mut child = {
            let _guard = runtime.enter();
            command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .map_err(|_| "MCP process failed to start")?
        };
        let input = child.stdin.take();
        let output = child.stdout.take().map(BufReader::new);
        Ok(Self {
            runtime: Some(runtime),
            child,
            input,
            output,
            timeout,
            closed: false,
            cancellation: sts2_harness::ExecutionCancellation::default(),
        })
    }
}
