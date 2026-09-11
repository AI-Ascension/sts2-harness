// SPDX-License-Identifier: MIT

use super::protocol::NativeFrame;
use super::types::{MAX_METHOD_BYTES, SessionError};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

#[path = "transport_calls.rs"]
mod calls;
#[path = "transport_config.rs"]
mod config;
#[path = "transport_io.rs"]
mod io;
use config::NativeProcessConfig;

const MAX_OUTSTANDING: usize = 16;
pub(super) const MAX_NATIVE_STATE_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeTransportError {
    NotInitialized,
    AlreadyInitialized,
    Capacity,
    Protocol,
    UnauthorizedServerRequest,
    Timeout,
    Unavailable,
    Closed,
    Ambiguous,
    Unsupported,
    Fenced,
}

impl std::fmt::Display for NativeTransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotInitialized => "native connection is not initialized",
            Self::AlreadyInitialized => "native connection was initialized twice",
            Self::Capacity => "native frame or queue exceeds its bound",
            Self::Protocol => "native frame is malformed",
            Self::UnauthorizedServerRequest => "native server request was denied",
            Self::Timeout => "native operation timed out",
            Self::Unavailable => "native process is unavailable",
            Self::Closed => "native connection is closed",
            Self::Ambiguous => "native operation outcome is ambiguous",
            Self::Unsupported => "native method is not allowlisted",
            Self::Fenced => "native connection is fenced",
        })
    }
}

impl std::error::Error for NativeTransportError {}

impl From<NativeTransportError> for SessionError {
    fn from(error: NativeTransportError) -> Self {
        match error {
            NativeTransportError::Capacity => Self::Capacity,
            NativeTransportError::UnauthorizedServerRequest => Self::Fenced,
            NativeTransportError::Ambiguous => Self::Ambiguous,
            NativeTransportError::Closed => Self::Closed,
            NativeTransportError::Protocol => Self::Protocol,
            NativeTransportError::NotInitialized
            | NativeTransportError::AlreadyInitialized
            | NativeTransportError::Timeout
            | NativeTransportError::Unavailable => Self::Transport,
            NativeTransportError::Unsupported => Self::Unsupported,
            NativeTransportError::Fenced => Self::Fenced,
        }
    }
}

/// Owned stdio worker for the bounded product envelope.  A server-initiated request is always
/// denied and fences the connection; no callback or command evaluator exists on this type.
pub struct OwnedNativeTransport {
    config: NativeProcessConfig,
    child: Option<Child>,
    writer: Option<BufWriter<ChildStdin>>,
    reader: Option<BufReader<ChildStdout>>,
    initialized: bool,
    closed: bool,
    fenced: bool,
    next_request_id: u64,
    outstanding: usize,
    notifications: usize,
    notification_sequences: BTreeSet<u64>,
}

impl std::fmt::Debug for OwnedNativeTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnedNativeTransport")
            .field("executable", &self.config.executable)
            .field("initialized", &self.initialized)
            .field("closed", &self.closed)
            .field("fenced", &self.fenced)
            .field("outstanding", &self.outstanding)
            .field("notifications", &self.notifications)
            .finish()
    }
}

impl OwnedNativeTransport {
    #[must_use]
    fn new(config: NativeProcessConfig) -> Self {
        Self {
            config,
            child: None,
            writer: None,
            reader: None,
            initialized: false,
            closed: false,
            fenced: false,
            next_request_id: 1,
            outstanding: 0,
            notifications: 0,
            notification_sequences: BTreeSet::new(),
        }
    }

    /// Construct the compiled offline peer without accepting a caller-selected executable,
    /// arguments, environment or credential.  The state root is a private, owner-restricted
    /// temporary directory so the fixture does not depend on the caller's checkout permissions.
    /// A real native profile must supply its own reviewed broker-owned factory rather than widening
    /// this fixture constructor.
    pub fn fixture_peer() -> Result<Self, NativeTransportError> {
        Ok(Self::new(Self::fixture_peer_config(
            private_fixture_state_root()?,
        )?))
    }

    /// Construct the compiled offline peer bound to an explicit state root, creating it with
    /// owner-only permissions when absent.  Tests use this to own and clean up their own root.
    pub fn fixture_peer_at(state_root: PathBuf) -> Result<Self, NativeTransportError> {
        Ok(Self::new(Self::fixture_peer_config(state_root)?))
    }

    fn fixture_peer_config(
        state_root: PathBuf,
    ) -> Result<NativeProcessConfig, NativeTransportError> {
        let executable = fixture_peer_executable().ok_or(NativeTransportError::Unavailable)?;
        prepare_private_fixture_root(&state_root)?;
        NativeProcessConfig::new(
            executable.to_string_lossy().into_owned(),
            Vec::new(),
            state_root.clone(),
            Vec::new(),
            state_root,
        )
        .map_err(|_| NativeTransportError::Unavailable)
    }

    /// The private state root the owned child is bound to.
    #[must_use]
    pub fn state_root(&self) -> &Path {
        self.config.state_root()
    }

    pub fn start(&mut self) -> Result<(), NativeTransportError> {
        if self.closed || self.child.is_some() {
            return Err(if self.closed {
                NativeTransportError::Closed
            } else {
                NativeTransportError::AlreadyInitialized
            });
        }
        if !io::safe_executable(PathBuf::from(self.config.executable()).as_path())
            || !io::safe_directory(self.config.working_directory())
            || !io::safe_directory(self.config.state_root())
            || !io::state_within_quota(self.config.state_root(), self.config.state_quota_bytes)
        {
            return Err(NativeTransportError::Unavailable);
        }
        let mut command = Command::new(self.config.executable());
        command
            .args(self.config.arguments())
            .current_dir(self.config.working_directory())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        self.config.apply_isolated_environment(&mut command);
        let mut child = command
            .spawn()
            .map_err(|_| NativeTransportError::Unavailable)?;
        let stdin = child
            .stdin
            .take()
            .ok_or(NativeTransportError::Unavailable)?;
        let stdout = child
            .stdout
            .take()
            .ok_or(NativeTransportError::Unavailable)?;
        self.writer = Some(BufWriter::new(stdin));
        self.reader = Some(BufReader::new(stdout));
        self.child = Some(child);
        Ok(())
    }

    pub fn initialize(&mut self) -> Result<Value, NativeTransportError> {
        if self.initialized {
            return Err(NativeTransportError::AlreadyInitialized);
        }
        let value = self.request(
            "initialize",
            json!({"tools": false, "ambient_history": false}),
        )?;
        self.initialized = true;
        Ok(value)
    }

    pub(crate) fn request(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<Value, NativeTransportError> {
        if self.closed {
            return Err(NativeTransportError::Closed);
        }
        if self.fenced {
            return Err(NativeTransportError::Fenced);
        }
        if self.child.is_none() {
            return Err(NativeTransportError::Unavailable);
        }
        if !io::state_within_quota(self.config.state_root(), self.config.state_quota_bytes) {
            return Err(self.fence_for_state_quota());
        }
        if !self.initialized && method != "initialize" {
            return Err(NativeTransportError::NotInitialized);
        }
        if method.is_empty()
            || method.len() > MAX_METHOD_BYTES
            || !valid_method(method)
            || self.outstanding >= MAX_OUTSTANDING
        {
            return Err(NativeTransportError::Capacity);
        }
        if !allowlisted_method(method) {
            return Err(NativeTransportError::Unsupported);
        }
        let id = self.next_request_id;
        self.next_request_id = self
            .next_request_id
            .checked_add(1)
            .ok_or(NativeTransportError::Capacity)?;
        let frame = NativeFrame::request(id, method, params);
        self.write_frame(&frame)?;
        self.outstanding = self.outstanding.saturating_add(1);
        let result = self.read_until(id);
        self.outstanding = self.outstanding.saturating_sub(1);
        if !io::state_within_quota(self.config.state_root(), self.config.state_quota_bytes) {
            return Err(self.fence_for_state_quota());
        }
        result
    }

    #[must_use]
    pub fn initialized(&self) -> bool {
        self.initialized
    }

    #[must_use]
    pub fn fenced(&self) -> bool {
        self.fenced
    }

    #[must_use]
    pub fn notification_count(&self) -> usize {
        self.notifications
    }

    pub fn close(&mut self) -> Result<(), NativeTransportError> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        self.writer = None;
        self.reader = None;
        if let Some(mut child) = self.child.take() {
            child
                .kill()
                .map_err(|_| NativeTransportError::Unavailable)?;
            let _ = child.wait();
        }
        Ok(())
    }

    fn fence_for_state_quota(&mut self) -> NativeTransportError {
        self.fenced = true;
        self.writer = None;
        self.reader = None;
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        NativeTransportError::Capacity
    }
}

impl Drop for OwnedNativeTransport {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

fn valid_method(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
}

fn allowlisted_method(value: &str) -> bool {
    matches!(
        value,
        "initialize"
            | "thread/start"
            | "thread/read"
            | "turn/start"
            | "turn/interrupt"
            | "thread/fork"
            | "thread/compact/start"
    )
}

static FIXTURE_ROOT_COUNTER: AtomicU64 = AtomicU64::new(1);

fn private_fixture_state_root() -> Result<PathBuf, NativeTransportError> {
    let counter = FIXTURE_ROOT_COUNTER.fetch_add(1, Ordering::Relaxed);
    Ok(std::env::temp_dir().join(format!(
        "ascension-provider-fixture-peer-{}-{counter}",
        std::process::id()
    )))
}

fn prepare_private_fixture_root(path: &Path) -> Result<(), NativeTransportError> {
    std::fs::create_dir_all(path).map_err(|_| NativeTransportError::Unavailable)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| NativeTransportError::Unavailable)?;
    }
    Ok(())
}

fn fixture_peer_executable() -> Option<PathBuf> {
    let current = std::env::current_exe().ok()?;
    let target_directory = current.parent()?.parent()?;
    let executable = target_directory.join("provider-session-peer");
    let canonical = std::fs::canonicalize(executable).ok()?;
    (!io::has_symlink_component(&canonical)
        && std::fs::symlink_metadata(&canonical)
            .is_ok_and(|metadata| metadata.file_type().is_file()))
    .then_some(canonical)
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
