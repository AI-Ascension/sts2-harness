// SPDX-License-Identifier: MIT

//! Safe Linux transport boundary for the authenticated harness worker handoff.
//!
//! This module owns the local endpoint, Linux peer proof, protected
//! credential read and bounded framing. It deliberately does not decode a
//! handoff command, access a store, admit work, or execute a process. The
//! parent harness server receives bounded bytes plus an opaque witness and
//! remains responsible for every semantic and durable decision.
//!
//! Root integration must add the Linux-only module/export and the
//! target-specific `rustix`/`tokio`/`subtle`/`zeroize` dependencies after
//! independent review. This preparation branch intentionally does not export
//! the module from the library.

#![cfg(target_os = "linux")]

#[path = "worker_local_linux_fs.rs"]
mod worker_local_linux_fs;
#[path = "worker_local_linux_process.rs"]
mod worker_local_linux_process;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rustix::fd::OwnedFd;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::time::{Instant, timeout_at};
use zeroize::Zeroizing;

use crate::worker_frame_io::{ConnectionDeadline, FrameIoError, WorkerFrameIo};
use crate::worker_handoff::MAX_FRAME_BYTES;
use worker_local_linux_fs::{HeldEndpoint, is_canonical_absolute};
use worker_local_linux_process::{HeldImage, ProtectedCredential, verify_peer};

/// The fixed authentication profile prefix, including its NUL terminator.
pub const AUTH_MAGIC: &[u8; 25] = b"ascension-worker-auth-v1\0";
/// Maximum private credential bytes accepted after [`AUTH_MAGIC`].
pub const MAX_CREDENTIAL_BYTES: usize = 4_096;
/// Maximum auth body after the four-byte big-endian body length.
pub const MAX_AUTH_BODY_BYTES: usize = AUTH_MAGIC.len() + MAX_CREDENTIAL_BYTES;

/// Fixed, redacted transport failures. No path, PID, OS error or secret is
/// included in a public error value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LinuxTransportError {
    Configuration,
    Peer,
    Credential,
    Deadline,
    Framing,
    Busy,
    Closed,
    Io,
}

impl std::fmt::Display for LinuxTransportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Configuration => "invalid Linux worker transport configuration",
            Self::Peer => "Linux worker peer authentication failed",
            Self::Credential => "Linux worker credential authentication failed",
            Self::Deadline => "Linux worker connection deadline exceeded",
            Self::Framing => "Linux worker frame rejected",
            Self::Busy => "Linux worker transport is busy",
            Self::Closed => "Linux worker connection closed",
            Self::Io => "Linux worker transport I/O failed",
        })
    }
}

impl std::error::Error for LinuxTransportError {}

impl From<FrameIoError> for LinuxTransportError {
    fn from(error: FrameIoError) -> Self {
        match error {
            FrameIoError::InvalidLimit | FrameIoError::InvalidLength => Self::Framing,
            FrameIoError::Deadline => Self::Deadline,
            FrameIoError::Transport => Self::Io,
            FrameIoError::Closed => Self::Closed,
        }
    }
}

/// Immutable process identity supplied by the owner-approved launch record.
/// It is never populated from a client frame.
pub struct LinuxPeerIdentity {
    uid: u32,
    pid: u32,
    start_token: u64,
    executable: PathBuf,
    executable_sha256: [u8; 32],
}

impl LinuxPeerIdentity {
    /// Creates an approved peer identity from owner-validated launch data.
    /// The executable path must be absolute and contain no traversal or dot
    /// components. The digest is the SHA-256 of the approved image.
    pub fn new(
        uid: u32,
        pid: u32,
        start_token: u64,
        executable: PathBuf,
        executable_sha256: [u8; 32],
    ) -> Result<Self, LinuxTransportError> {
        if pid == 0 || start_token == 0 || !is_canonical_absolute(&executable) {
            return Err(LinuxTransportError::Configuration);
        }
        Ok(Self {
            uid,
            pid,
            start_token,
            executable,
            executable_sha256,
        })
    }
}

/// Owner-local endpoint and credential configuration.
pub struct LinuxWorkerConfig {
    endpoint: PathBuf,
    credential: PathBuf,
    peer: LinuxPeerIdentity,
}

impl LinuxWorkerConfig {
    /// Constructs a configuration without opening any path or accepting a
    /// connection. Opening and identity checks happen in [`Self::bind`].
    pub fn new(
        endpoint: PathBuf,
        credential: PathBuf,
        peer: LinuxPeerIdentity,
    ) -> Result<Self, LinuxTransportError> {
        if endpoint == credential
            || !is_canonical_absolute(&endpoint)
            || !is_canonical_absolute(&credential)
        {
            return Err(LinuxTransportError::Configuration);
        }
        Ok(Self {
            endpoint,
            credential,
            peer,
        })
    }

    /// Binds the first owner-only Unix listener and reads the protected
    /// credential through a held descriptor. Existing endpoint names are
    /// never removed or replaced.
    pub fn bind(self) -> Result<LinuxWorkerListener, LinuxTransportError> {
        LinuxWorkerListener::bind(self)
    }
}

/// An authenticated, one-request/one-response Linux worker listener.
pub struct LinuxWorkerListener {
    listener: UnixListener,
    endpoint: HeldEndpoint,
    credential: ProtectedCredential,
    image: HeldImage,
    peer: LinuxPeerIdentity,
    active: Arc<AtomicBool>,
}

impl LinuxWorkerListener {
    fn bind(config: LinuxWorkerConfig) -> Result<Self, LinuxTransportError> {
        let owner_uid = rustix::process::getuid().as_raw();
        let credential = ProtectedCredential::open(&config.credential, owner_uid)?;
        let image = HeldImage::open(
            &config.peer.executable,
            owner_uid,
            Some(&config.peer.executable_sha256),
        )
        .map_err(|_| LinuxTransportError::Configuration)?;
        let mut endpoint = HeldEndpoint::bind(&config.endpoint, owner_uid)?;
        Ok(Self {
            listener: endpoint.listener()?,
            endpoint,
            credential,
            image,
            peer: config.peer,
            active: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Accepts one connection and authenticates it before exposing any frame
    /// bytes. `deadline` must be created before this call; it is reused for
    /// accept, peer proof, auth prelude and JSON framing.
    pub async fn accept_authenticated(
        &self,
        deadline: ConnectionDeadline,
    ) -> Result<AuthenticatedWorkerConnection, LinuxTransportError> {
        let exchange = ExchangeGuard::acquire(&self.active)?;
        let instant = deadline.instant();
        self.endpoint.verify_path()?;
        let (stream, _) = timeout_at(instant, self.listener.accept())
            .await
            .map_err(|_| LinuxTransportError::Deadline)?
            .map_err(|_| LinuxTransportError::Io)?;
        if Instant::now() >= instant {
            return Err(LinuxTransportError::Deadline);
        }
        self.endpoint.verify_path()?;
        let witness = verify_peer(&stream, &self.peer, &self.image, instant)?;
        let mut stream = stream;
        authenticate_credential(&mut stream, &self.credential, instant).await?;
        if Instant::now() >= instant {
            return Err(LinuxTransportError::Deadline);
        }
        Ok(AuthenticatedWorkerConnection {
            io: WorkerFrameIo::new(stream, deadline),
            witness,
            _exchange: exchange,
            request_read: false,
            response_written: false,
        })
    }
}

/// Opaque proof that the connected peer passed kernel credentials, pidfd,
/// start-token, executable path and executable-digest checks. The type is not
/// serializable, cloneable or constructible outside this module.
pub struct LinuxPeerWitness {
    _pidfd: OwnedFd,
    _pid: u32,
    _uid: u32,
    _image: HeldImage,
}

/// One authenticated connection. It owns the stream and held peer fd;
/// dropping it closes both. It permits exactly one request followed by one
/// response and cannot resume a failed or cancelled operation.
pub struct AuthenticatedWorkerConnection {
    io: WorkerFrameIo<UnixStream>,
    witness: LinuxPeerWitness,
    _exchange: ExchangeGuard,
    request_read: bool,
    response_written: bool,
}

struct ExchangeGuard(Arc<AtomicBool>);

impl ExchangeGuard {
    fn acquire(active: &Arc<AtomicBool>) -> Result<Self, LinuxTransportError> {
        active
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map(|_| Self(Arc::clone(active)))
            .map_err(|_| LinuxTransportError::Busy)
    }
}

impl Drop for ExchangeGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl AuthenticatedWorkerConnection {
    /// Returns the non-forgeable proof witness for parent-side capability and
    /// durable admission checks. No PID or credential accessor is exposed.
    pub fn peer_witness(&self) -> &LinuxPeerWitness {
        &self.witness
    }

    /// Reads exactly one bounded handoff request frame. The caller owns JSON
    /// decoding and all semantic/admission decisions.
    pub async fn read_request_bytes(&mut self) -> Result<Vec<u8>, LinuxTransportError> {
        if self.request_read || self.response_written {
            return Err(LinuxTransportError::Closed);
        }
        let result = self.io.read_frame(MAX_FRAME_BYTES).await;
        self.request_read = true;
        result.map_err(Into::into)
    }

    /// Writes exactly one bounded handoff response frame after the request has
    /// been read. This performs no response decoding or authorization.
    pub async fn write_response_bytes(
        &mut self,
        response: &[u8],
    ) -> Result<(), LinuxTransportError> {
        if !self.request_read || self.response_written {
            return Err(LinuxTransportError::Closed);
        }
        let result = self.io.write_frame(response, MAX_FRAME_BYTES).await;
        self.response_written = true;
        result.map_err(Into::into)
    }
}

async fn authenticate_credential(
    stream: &mut UnixStream,
    credential: &ProtectedCredential,
    deadline: Instant,
) -> Result<(), LinuxTransportError> {
    let mut length = [0_u8; 4];
    read_exact_until(stream, &mut length, deadline).await?;
    let body_length =
        usize::try_from(u32::from_be_bytes(length)).map_err(|_| LinuxTransportError::Credential)?;
    if !(AUTH_MAGIC.len() + 1..=MAX_AUTH_BODY_BYTES).contains(&body_length) {
        return Err(LinuxTransportError::Credential);
    }
    let mut body = Zeroizing::new(vec![0_u8; body_length]);
    read_exact_until(stream, body.as_mut_slice(), deadline).await?;
    if body.get(..AUTH_MAGIC.len()) != Some(AUTH_MAGIC.as_slice()) {
        return Err(LinuxTransportError::Credential);
    }
    if !credential.matches(&body[AUTH_MAGIC.len()..]) {
        return Err(LinuxTransportError::Credential);
    }
    Ok(())
}

async fn read_exact_until<R: AsyncRead + Unpin>(
    reader: &mut R,
    bytes: &mut [u8],
    deadline: Instant,
) -> Result<(), LinuxTransportError> {
    timeout_at(deadline, reader.read_exact(bytes))
        .await
        .map_err(|_| LinuxTransportError::Deadline)?
        .map(|_| ())
        .map_err(|_| LinuxTransportError::Closed)
}
