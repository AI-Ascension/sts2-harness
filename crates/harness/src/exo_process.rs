// SPDX-License-Identifier: MIT

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, TryRecvError};
use std::time::{Duration, Instant};

use crate::exo::{ExoTransport, ExoTransportError};

const MAX_EXECUTABLE_BYTES: usize = 1_024;
const MAX_ARGUMENTS: usize = 32;
const MAX_ARGUMENT_BYTES: usize = 2_048;
const MAX_ENVIRONMENT_NAMES: usize = 32;
const MAX_ENVIRONMENT_NAME_BYTES: usize = 128;

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
            || config.working_directory.as_ref().is_some_and(|path| {
                !valid_path(path, MAX_EXECUTABLE_BYTES)
            })
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
        let mut command = Command::new(&self.config.executable);
        command
            .args(&self.config.arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .env_clear();
        if let Some(directory) = &self.config.working_directory {
            command.current_dir(directory);
        }
        for name in &self.config.inherited_environment {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        let mut child = command
            .spawn()
            .map_err(|_| ExoTransportError::Unavailable)?;
        let mut input = child.stdin.take().ok_or_else(|| {
            terminate(&mut child);
            ExoTransportError::Unavailable
        })?;
        if input.write_all(request).is_err() {
            terminate(&mut child);
            return Err(ExoTransportError::Unavailable);
        }
        drop(input);
        let output = child.stdout.take().ok_or_else(|| {
            terminate(&mut child);
            ExoTransportError::Unavailable
        })?;
        let (sender, receiver) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let result = read_bounded(output, max_response_bytes);
            match sender.send(result) {
                Ok(()) | Err(_) => {}
            }
        });

        let timeout = Duration::from_millis(u64::from(timeout_millis));
        let deadline = Instant::now() + timeout;
        let mut response = None;
        let mut status = None;
        loop {
            if response.is_none() {
                match receiver.try_recv() {
                    Ok(Ok(bytes)) => response = Some(Ok(bytes)),
                    Ok(Err(error)) => {
                        terminate(&mut child);
                        return Err(error);
                    }
                    Err(TryRecvError::Disconnected) => {
                        terminate(&mut child);
                        return Err(ExoTransportError::Unavailable);
                    }
                    Err(TryRecvError::Empty) => {}
                }
            }
            if status.is_none() {
                status = match child.try_wait() {
                    Ok(status) => status,
                    Err(_) => {
                        terminate(&mut child);
                        return Err(ExoTransportError::Unavailable);
                    }
                };
            }
            if let (Some(response), Some(status)) = (response.as_ref(), status.as_ref()) {
                if !status.success() {
                    return Err(ExoTransportError::Unavailable);
                }
                return match response {
                    Ok(bytes) if bytes.is_empty() => Err(ExoTransportError::MalformedResponse),
                    Ok(bytes) => Ok(bytes.clone()),
                    Err(error) => Err(*error),
                };
            }
            if Instant::now() >= deadline {
                terminate(&mut child);
                return Err(ExoTransportError::Timeout);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn close(&mut self) -> Result<(), ExoTransportError> {
        self.closed = true;
        Ok(())
    }
}

fn read_bounded(mut output: impl Read, maximum: usize) -> Result<Vec<u8>, ExoTransportError> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4_096];
    loop {
        let count = output
            .read(&mut chunk)
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

fn terminate(child: &mut Child) {
    match child.kill() {
        Ok(()) | Err(_) => {}
    }
    match child.wait() {
        Ok(_) | Err(_) => {}
    }
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
