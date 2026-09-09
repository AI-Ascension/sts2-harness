// SPDX-License-Identifier: MIT

use serde_json::Value;

use super::super::config::RuntimeConfig;
use super::McpProcess;

impl McpProcess {
    pub(in super::super) fn spawn_with_cancellation(
        config: &RuntimeConfig,
        cancellation: &sts2_harness::ExecutionCancellation,
    ) -> Result<Self, String> {
        if cancellation.is_cancelled() {
            return Err(String::from("MCP launch cancelled"));
        }
        let mut process = Self::spawn_command(
            Self::configured_command(config),
            std::time::Duration::from_secs(5),
        )?;
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
        let mut process = Self::spawn_command(command, std::time::Duration::from_secs(5))?;
        process.cancellation = cancellation.clone();
        Ok(process)
    }
}
