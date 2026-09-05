// SPDX-License-Identifier: MIT

use std::process::Stdio;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use super::config::RuntimeConfig;

const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_REQUEST_BYTES: usize = 64 * 1024;
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(5);
const CLEANUP_TIMEOUT: Duration = Duration::from_millis(250);

pub(super) struct McpProcess {
    runtime: Option<tokio::runtime::Runtime>,
    child: Child,
    input: Option<ChildStdin>,
    output: Option<BufReader<ChildStdout>>,
    timeout: Duration,
    closed: bool,
}

impl McpProcess {
    pub(super) fn spawn(config: &RuntimeConfig) -> Result<Self, String> {
        Self::spawn_command(Self::configured_command(config), EXCHANGE_TIMEOUT)
    }

    fn configured_command(config: &RuntimeConfig) -> Command {
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
        let deadline = Instant::now()
            .checked_add(timeout)
            .filter(|_| !timeout.is_zero())
            .ok_or_else(|| String::from("MCP exchange deadline is invalid"))?;
        if self.closed {
            return Err(String::from("MCP process is closed"));
        }
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let mut bytes = serde_json::to_vec(&request)
            .map_err(|_| String::from("MCP request serialization failed"))?;
        bytes.push(b'\n');
        if bytes.len() > MAX_REQUEST_BYTES {
            return Err(String::from("MCP request exceeded its size limit"));
        }
        let runtime = self.runtime.as_ref().ok_or("MCP supervisor is closed")?;
        let result = supervised(|| {
            runtime.block_on(async {
                let input = self.input.as_mut().ok_or("MCP stdin is closed")?;
                let output = self.output.as_mut().ok_or("MCP stdout is closed")?;
                tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), async {
                    let write = async {
                        input
                            .write_all(&bytes)
                            .await
                            .map_err(|_| "MCP request write failed")?;
                        input.flush().await.map_err(|_| "MCP request flush failed")
                    };
                    let (_, response) = tokio::try_join!(write, read_frame(output))?;
                    validate_response(&response, id)
                })
                .await
                .map_err(|_| "MCP exchange timed out")?
            })
        });
        if let Err(error) = result {
            return match self.terminate() {
                Ok(()) => Err(error),
                Err(cleanup) => Err(format!("{error}; {cleanup}")),
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
                tokio::time::timeout(CLEANUP_TIMEOUT, self.child.wait())
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
                let status = tokio::time::timeout(CLEANUP_TIMEOUT, self.child.wait())
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
            return match self.terminate() {
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
fn supervised<T: Send>(work: impl FnOnce() -> Result<T, &'static str> + Send) -> Result<T, String> {
    std::thread::scope(|scope| {
        let worker = std::thread::Builder::new()
            .name(String::from("mcp-exchange"))
            .spawn_scoped(scope, work)
            .map_err(|_| String::from("MCP supervisor unavailable"))?;
        worker
            .join()
            .map_err(|_| String::from("MCP supervisor failed"))?
            .map_err(String::from)
    })
}

async fn read_frame(output: &mut BufReader<ChildStdout>) -> Result<Vec<u8>, &'static str> {
    let mut bytes = Vec::new();
    loop {
        let available = output
            .fill_buf()
            .await
            .map_err(|_| "MCP response read failed")?;
        if available.is_empty() {
            return Err("MCP response ended before its delimiter");
        }
        let delimiter = available.iter().position(|byte| *byte == b'\n');
        let count = delimiter.map_or(available.len(), |index| index + 1);
        if bytes.len().saturating_add(count) > MAX_RESPONSE_BYTES {
            return Err("MCP response exceeded its size limit");
        }
        bytes.extend_from_slice(&available[..count]);
        output.consume(count);
        if delimiter.is_some() {
            return Ok(bytes);
        }
    }
}

fn validate_response(bytes: &[u8], id: u64) -> Result<Value, &'static str> {
    let response: Value = serde_json::from_slice(bytes).map_err(|_| "MCP response was not JSON")?;
    if !response.is_object()
        || response["jsonrpc"] != "2.0"
        || response["id"].as_u64() != Some(id)
        || response.get("result").is_some() == response.get("error").is_some()
    {
        return Err("MCP response envelope was invalid");
    }
    Ok(response)
}

#[cfg(test)]
#[path = "mcp_process_tests.rs"]
mod tests;
