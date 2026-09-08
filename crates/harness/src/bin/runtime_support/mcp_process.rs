// SPDX-License-Identifier: MIT

use std::process::Stdio;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use super::config::RuntimeConfig;

const MAX_RESPONSE_BYTES: usize = 256 * 1024;
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
    closed: bool,
}

impl McpProcess {
    #[cfg(test)]
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
        Self::spawn_command(
            Self::configured_command_for_profile(config, profile),
            EXCHANGE_TIMEOUT,
        )
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

    fn spawn_command(mut command: Command, timeout: Duration) -> Result<Self, String> {
        supervised(|| Self::spawn_supervised(&mut command, timeout))
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
                    let (_, response) = tokio::try_join!(write, read_frame(output))?;
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

    fn terminate(&mut self) -> Result<(), String> {
        self.closed = true;
        self.input.take();
        self.output.take();
        let runtime = self.runtime.as_ref().ok_or("MCP supervisor is closed")?;
        supervised(|| {
            runtime.block_on(async {
                self.child
                    .start_kill()
                    .map_err(|_| "MCP termination failed")?;
                tokio::time::timeout(FORCE_REAP_TIMEOUT, self.child.wait())
                    .await
                    .map_err(|_| "MCP reap timed out")?
                    .map_err(|_| "MCP reap failed")?;
                Ok(())
            })
        })
    }

    pub(super) fn close(&mut self) -> Result<(), String> {
        if self.closed {
            return if self.child.id().is_some() {
                self.terminate()
            } else {
                Ok(())
            };
        }
        self.input.take();
        self.output.take();
        let runtime = self.runtime.as_ref().ok_or("MCP supervisor is closed")?;
        let result = supervised(|| {
            runtime.block_on(async {
                let status = tokio::time::timeout(GRACEFUL_CLOSE_TIMEOUT, self.child.wait())
                    .await
                    .map_err(|_| "MCP shutdown timed out")?
                    .map_err(|_| "MCP process wait failed")?;
                if status.success() {
                    Ok(())
                } else {
                    Err("MCP process exited unsuccessfully")
                }
            })
        });
        if let Err(error) = result {
            let cleanup = self.terminate();
            return match cleanup {
                Ok(()) => Err(error),
                Err(cleanup) => Err(format!("{error}; {cleanup}")),
            };
        }
        self.closed = true;
        result
    }
}

impl Drop for McpProcess {
    fn drop(&mut self) {
        if self.child.id().is_some() {
            let _cleanup = self.terminate();
        }
        if let Some(runtime) = self.runtime.take() {
            let _cleanup = supervised(|| {
                drop(runtime);
                Ok(())
            });
        }
    }
}

// No detached workers: cancellation drops asynchronous pipe futures, including when a
// descendant retains inherited descriptors. Every scoped supervisor is joined before return.
include!("mcp_process_transport.rs");

#[cfg(test)]
#[path = "mcp_process_tests.rs"]
mod tests;
