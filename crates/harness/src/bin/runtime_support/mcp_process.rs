// SPDX-License-Identifier: MIT

use std::process::Stdio;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use super::config::RuntimeConfig;

const MAX_RESPONSE_BYTES: usize = 256 * 1024;
// A complete map snapshot is bounded at 256 KiB before MCP wraps it as JSON text. The map profile
// needs room for that escaped text and the JSON-RPC/content envelope while remaining bounded.
const MAP_MAX_RESPONSE_BYTES: usize = 512 * 1024;
const MAX_REQUEST_BYTES: usize = 256 * 1024;
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(5);
// A child may still be finishing a response or flushing its own shutdown work after the
// caller has closed its pipes. Give that graceful path a bounded second before treating it
// as a failed shutdown. The force-reap budget remains short so a stalled child cannot hold
// the supervisor indefinitely after the graceful path has failed.
const GRACEFUL_CLOSE_TIMEOUT: Duration = Duration::from_secs(1);
const FORCE_REAP_TIMEOUT: Duration = Duration::from_millis(250);

include!("mcp_process_error.rs");

pub(super) struct McpProcess {
    runtime: Option<tokio::runtime::Runtime>,
    child: Child,
    input: Option<ChildStdin>,
    output: Option<BufReader<ChildStdout>>,
    timeout: Duration,
    max_response_bytes: usize,
    closed: bool,
}

impl McpProcess {
    #[cfg(all(test, unix))]
    pub(super) const fn is_closed(&self) -> bool {
        self.closed
    }

    pub(super) fn refresh_closed(&mut self) -> bool {
        if self.closed {
            return true;
        }
        if self.child.try_wait().ok().flatten().is_some() {
            self.closed = true;
        }
        self.closed
    }

    pub(super) fn spawn(config: &RuntimeConfig) -> Result<Self, String> {
        Self::spawn_command(Self::configured_command(config), EXCHANGE_TIMEOUT)
    }

    pub(super) fn spawn_profile(config: &RuntimeConfig, profile: &str) -> Result<Self, String> {
        let max_response_bytes = if profile == "runtime-map-v1" {
            MAP_MAX_RESPONSE_BYTES
        } else {
            MAX_RESPONSE_BYTES
        };
        Self::spawn_command_with_response_limit(
            Self::configured_command_for_profile(config, profile),
            EXCHANGE_TIMEOUT,
            max_response_bytes,
        )
    }

    pub(super) fn spawn_recovery(
        config: &RuntimeConfig,
        instance_id: &str,
        lease_id: &str,
        lease_epoch: u64,
        recovery_environment: &[(String, String)],
    ) -> Result<Self, String> {
        let mut command = Self::configured_command_for_profile(config, "watchdog-recovery-v1");
        command
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
        for (name, value) in recovery_environment {
            command.env(name, value);
        }
        if config.recovery_value("STS2_RECOVERY_TOKEN").is_none() {
            return Err(String::from(
                "STS2_RECOVERY_TOKEN is required for the recovery sideband",
            ));
        }
        Self::spawn_command(command, EXCHANGE_TIMEOUT)
    }

    fn configured_command(config: &RuntimeConfig) -> Command {
        Self::configured_command_for_profile(config, &config.runtime_profile)
    }

    fn configured_command_for_profile(config: &RuntimeConfig, profile: &str) -> Command {
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
            .env("STS2_RUNTIME_PROFILE", profile)
            .env("STS2_INSTANCE_ID", &config.instance_id)
            .env("STS2_CALLER_ID", &config.caller_id)
            .env("STS2_SESSION_ID", &config.session_id)
            .env("STS2_MCP_SESSION_ID", &config.mcp_session_id)
            .env("STS2_LEASE_ID", &config.lease_id)
            .env("STS2_LEASE_EPOCH", config.lease_epoch.to_string());
        command
    }

    fn spawn_command(command: Command, timeout: Duration) -> Result<Self, String> {
        Self::spawn_command_with_response_limit(command, timeout, MAX_RESPONSE_BYTES)
    }

    fn spawn_command_with_response_limit(
        mut command: Command,
        timeout: Duration,
        max_response_bytes: usize,
    ) -> Result<Self, String> {
        supervised(|| Self::spawn_supervised(&mut command, timeout, max_response_bytes))
    }

    fn spawn_supervised(
        command: &mut Command,
        timeout: Duration,
        max_response_bytes: usize,
    ) -> Result<Self, &'static str> {
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
            max_response_bytes,
            closed: false,
        })
    }

    pub(super) fn call(&mut self, id: u64, method: &str, params: Value) -> Result<Value, String> {
        self.call_with_timeout(id, method, params, self.timeout)
    }

    pub(super) fn call_with_timeout(
        &mut self,
        id: u64,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        self.call_with_timeout_classified(id, method, params, timeout)
            .map_err(|error| error.to_string())
    }

    pub(super) fn call_with_timeout_classified(
        &mut self,
        id: u64,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, McpProcessError> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .filter(|_| !timeout.is_zero())
            .ok_or_else(|| {
                McpProcessError::new(
                    McpProcessErrorKind::Protocol,
                    "MCP exchange deadline is invalid",
                )
            })?;
        if self.closed {
            return Err(McpProcessError::new(
                McpProcessErrorKind::ProcessClosed,
                "MCP process is closed",
            ));
        }
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let mut bytes = serde_json::to_vec(&request).map_err(|_| {
            McpProcessError::new(
                McpProcessErrorKind::Protocol,
                "MCP request serialization failed",
            )
        })?;
        bytes.push(b'\n');
        if bytes.len() > MAX_REQUEST_BYTES {
            return Err(McpProcessError::new(
                McpProcessErrorKind::Protocol,
                "MCP request exceeded its size limit",
            ));
        }
        let runtime = self.runtime.as_ref().ok_or_else(|| {
            McpProcessError::new(
                McpProcessErrorKind::SupervisorClosed,
                "MCP supervisor is closed",
            )
        })?;
        let result = supervised_call(|| {
            runtime.block_on(async {
                let input = self.input.as_mut().ok_or_else(|| {
                    McpProcessError::new(McpProcessErrorKind::StdinClosed, "MCP stdin is closed")
                })?;
                let output = self.output.as_mut().ok_or_else(|| {
                    McpProcessError::new(McpProcessErrorKind::StdoutClosed, "MCP stdout is closed")
                })?;
                tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), async {
                    let write = async {
                        input.write_all(&bytes).await.map_err(|_| {
                            McpProcessError::new(
                                McpProcessErrorKind::RequestWrite,
                                "MCP request write failed",
                            )
                        })?;
                        input.flush().await.map_err(|_| {
                            McpProcessError::new(
                                McpProcessErrorKind::RequestFlush,
                                "MCP request flush failed",
                            )
                        })
                    };
                    let (_, response) =
                        tokio::try_join!(write, read_frame(output, self.max_response_bytes))?;
                    validate_response(&response, id)
                })
                .await
                .map_err(|_| {
                    McpProcessError::new(McpProcessErrorKind::Deadline, "MCP exchange timed out")
                })?
            })
        });
        if let Err(error) = result {
            return match self.terminate() {
                Ok(()) => Err(error),
                Err(cleanup) => Err(error.with_cleanup(cleanup)),
            };
        }
        result
    }
}

include!("mcp_process_lifecycle.rs");

// No detached workers: cancellation drops asynchronous pipe futures, including when a
// descendant retains inherited descriptors. Every scoped supervisor is joined before return.
include!("mcp_process_transport.rs");

#[cfg(test)]
#[path = "mcp_process_tests.rs"]
mod tests;
