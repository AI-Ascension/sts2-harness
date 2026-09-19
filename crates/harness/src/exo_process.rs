// SPDX-License-Identifier: MIT

use std::collections::BTreeSet;
use std::process::{ExitStatus, Stdio};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};

use crate::exo::{ExoTransport, ExoTransportError};

const MAX_EXECUTABLE_BYTES: usize = 1_024;
const MAX_ARGUMENTS: usize = 32;
const MAX_ARGUMENT_BYTES: usize = 2_048;
const MAX_ENVIRONMENT_NAMES: usize = 32;
const MAX_ENVIRONMENT_NAME_BYTES: usize = 128;
/// Bounded tail of a failed child's stderr kept for the operator-visible diagnostic.
const MAX_CHILD_STDERR_BYTES: usize = 4_096;

/// Operator-owned process configuration for an Exo bridge.
///
/// Arguments are passed directly to the executable and the decision request is written to stdin;
/// no shell is involved. Environment values are never stored in this configuration: only an
/// explicit allowlist of names may be inherited by the bridge process.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExoProcessConfig {
    executable: String,
    arguments: Vec<String>,
    working_directory: Option<String>,
    inherited_environment: Vec<String>,
}

impl ExoProcessConfig {
    pub fn new(
        executable: impl Into<String>,
        arguments: Vec<String>,
        working_directory: Option<String>,
        inherited_environment: Vec<String>,
    ) -> Result<Self, ExoProcessConfigError> {
        let config = Self {
            executable: executable.into(),
            arguments,
            working_directory,
            inherited_environment,
        };
        if !valid_path(&config.executable, MAX_EXECUTABLE_BYTES)
            || config.arguments.len() > MAX_ARGUMENTS
            || config
                .arguments
                .iter()
                .any(|argument| !valid_path(argument, MAX_ARGUMENT_BYTES))
            || config
                .working_directory
                .as_ref()
                .is_some_and(|path| !valid_path(path, MAX_EXECUTABLE_BYTES))
            || config.inherited_environment.len() > MAX_ENVIRONMENT_NAMES
            || !valid_environment_names(&config.inherited_environment)
        {
            return Err(ExoProcessConfigError::Invalid);
        }
        Ok(config)
    }

    #[must_use]
    pub fn executable(&self) -> &str {
        &self.executable
    }

    #[must_use]
    pub fn arguments(&self) -> &[String] {
        &self.arguments
    }

    #[must_use]
    pub fn working_directory(&self) -> Option<&str> {
        self.working_directory.as_deref()
    }

    #[must_use]
    pub fn inherited_environment(&self) -> &[String] {
        &self.inherited_environment
    }
}

/// Configuration validation failure for the operator-owned process seam.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExoProcessConfigError {
    Invalid,
}

impl std::fmt::Display for ExoProcessConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Exo process configuration is invalid")
    }
}

impl std::error::Error for ExoProcessConfigError {}

/// Synchronous, bounded process transport for a small operator-owned Exo bridge.
#[derive(Debug)]
pub struct ExoProcessTransport {
    config: ExoProcessConfig,
    closed: bool,
}

impl ExoProcessTransport {
    #[must_use]
    pub fn new(config: ExoProcessConfig) -> Self {
        Self {
            config,
            closed: false,
        }
    }

    #[must_use]
    pub fn config(&self) -> &ExoProcessConfig {
        &self.config
    }
}

impl ExoTransport for ExoProcessTransport {
    fn exchange(
        &mut self,
        request: &[u8],
        max_response_bytes: usize,
        timeout_millis: u32,
    ) -> Result<Vec<u8>, ExoTransportError> {
        if self.closed || request.is_empty() || max_response_bytes == 0 || timeout_millis == 0 {
            return Err(ExoTransportError::MalformedResponse);
        }
        let deadline = Instant::now() + Duration::from_millis(u64::from(timeout_millis));
        // A joined supervisor permits synchronous callers inside or outside an async runtime.
        // Pipe I/O stays asynchronous: descendants cannot strand a blocking read/write worker.
        std::thread::scope(|scope| {
            let worker = std::thread::Builder::new()
                .name(String::from("exo-exchange"))
                .spawn_scoped(scope, || {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|_| ExoTransportError::Unavailable)?;
                    runtime.block_on(exchange_process(
                        &self.config,
                        request,
                        max_response_bytes,
                        deadline,
                    ))
                })
                .map_err(|_| ExoTransportError::Unavailable)?;
            worker.join().map_err(|_| ExoTransportError::Unavailable)?
        })
    }

    fn close(&mut self) -> Result<(), ExoTransportError> {
        self.closed = true;
        Ok(())
    }
}

async fn exchange_process(
    config: &ExoProcessConfig,
    request: &[u8],
    maximum: usize,
    deadline: Instant,
) -> Result<Vec<u8>, ExoTransportError> {
    if Instant::now() >= deadline {
        return Err(ExoTransportError::Timeout);
    }
    let mut command = Command::new(&config.executable);
    command
        .args(&config.arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .kill_on_drop(true);
    if let Some(directory) = &config.working_directory {
        command.current_dir(directory);
    }
    for name in &config.inherited_environment {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let mut child = command.spawn().map_err(|error| {
        eprintln!("{}", start_failure_diagnostic(&error));
        ExoTransportError::Unavailable
    })?;
    let result = tokio::time::timeout_at(
        tokio::time::Instant::from_std(deadline),
        exchange_pipes(&mut child, request, maximum),
    )
    .await
    .unwrap_or(Err(ExoTransportError::Timeout));
    if result.is_err() {
        // The timeout has dropped both pipe futures and their handles before cleanup begins.
        terminate(&mut child).await?;
    }
    result
}

async fn exchange_pipes(
    child: &mut Child,
    request: &[u8],
    maximum: usize,
) -> Result<Vec<u8>, ExoTransportError> {
    let mut input = child.stdin.take().ok_or(ExoTransportError::Unavailable)?;
    let output = child.stdout.take().ok_or(ExoTransportError::Unavailable)?;
    let errors = child.stderr.take().ok_or(ExoTransportError::Unavailable)?;
    let write = async move {
        input
            .write_all(request)
            .await
            .map_err(|_| ExoTransportError::Unavailable)?;
        input
            .shutdown()
            .await
            .map_err(|_| ExoTransportError::Unavailable)
    };
    let wait = async {
        child
            .wait()
            .await
            .map_err(|_| ExoTransportError::Unavailable)
    };
    // Every stream is polled to completion rather than short-circuited. The exit status and the
    // child's stderr are what name a transport that could not start, and a piped stderr left
    // unread would block such a child on a full pipe instead of letting it exit.
    let (written, response, status, errors) = tokio::join!(
        write,
        read_bounded(output, maximum),
        wait,
        read_tail(errors, MAX_CHILD_STDERR_BYTES)
    );
    let status = status?;
    if !status.success() {
        eprintln!("{}", child_failure_diagnostic(&status, &errors));
        return Err(ExoTransportError::Unavailable);
    }
    written?;
    let response = response?;
    if response.is_empty() {
        Err(ExoTransportError::MalformedResponse)
    } else {
        Ok(response)
    }
}

/// Reads a child stream to its end and keeps only the last `maximum` bytes.
///
/// The tail is a diagnostic, so it must neither fail the exchange nor grow without bound, and the
/// stream has to keep draining or a verbose child cannot exit.
async fn read_tail(mut stream: impl AsyncRead + Unpin, maximum: usize) -> Vec<u8> {
    let mut kept = Vec::new();
    let mut chunk = [0_u8; 4_096];
    loop {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return kept,
            Ok(count) => {
                kept.extend_from_slice(&chunk[..count]);
                if kept.len() > maximum {
                    let excess = kept.len() - maximum;
                    kept.drain(..excess);
                }
            }
        }
    }
}

/// Names why a transport child failed, using only what is bounded and safe to print.
fn child_failure_diagnostic(status: &ExitStatus, tail: &[u8]) -> String {
    let tail = String::from_utf8_lossy(tail);
    let tail = tail.trim_end();
    if tail.is_empty() {
        format!("exo transport child failed with {status} and wrote no stderr")
    } else {
        format!("exo transport child failed with {status}; stderr tail:\n{tail}")
    }
}

/// Names why a transport child could not be started at all.
fn start_failure_diagnostic(error: &std::io::Error) -> String {
    format!("exo transport child could not be started: {error}")
}

async fn read_bounded(
    mut output: impl AsyncRead + Unpin,
    maximum: usize,
) -> Result<Vec<u8>, ExoTransportError> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4_096];
    loop {
        let count = output
            .read(&mut chunk)
            .await
            .map_err(|_| ExoTransportError::Unavailable)?;
        if count == 0 {
            return Ok(bytes);
        }
        if bytes.len().saturating_add(count) > maximum {
            return Err(ExoTransportError::OversizedResponse);
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
}

async fn terminate(child: &mut Child) -> Result<(), ExoTransportError> {
    child
        .start_kill()
        .map_err(|_| ExoTransportError::Unavailable)?;
    // Reaping has its own bounded grace period, not an unbounded synchronous wait.
    tokio::time::timeout(Duration::from_millis(250), child.wait())
        .await
        .map_err(|_| ExoTransportError::Unavailable)?
        .map_err(|_| ExoTransportError::Unavailable)?;
    Ok(())
}

fn valid_path(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && !value.chars().any(char::is_control)
        && !value.contains('\0')
}

fn valid_environment_names(names: &[String]) -> bool {
    let mut unique = BTreeSet::new();
    names.iter().all(|name| {
        !name.is_empty()
            && name.len() <= MAX_ENVIRONMENT_NAME_BYTES
            && name.bytes().enumerate().all(|(index, byte)| {
                if index == 0 {
                    byte.is_ascii_alphabetic() || byte == b'_'
                } else {
                    byte.is_ascii_alphanumeric() || byte == b'_'
                }
            })
            && !matches!(
                name.as_str(),
                "LD_PRELOAD" | "LD_LIBRARY_PATH" | "DYLD_INSERT_LIBRARIES"
            )
            && unique.insert(name)
    })
}

#[cfg(test)]
mod tests {
    include!("exo_process_tests.rs");
}
