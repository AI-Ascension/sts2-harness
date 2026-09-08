// SPDX-License-Identifier: MIT

//! Safe Linux transport boundary for the authenticated harness worker handoff.
//!
//! This module owns the local endpoint, Linux peer proof, protected
//! credential read and bounded framing. It deliberately does not decode a
//! handoff command, access a store, admit work, or execute a process. The
//! parent harness server receives bounded bytes plus an opaque witness and
//! remains responsible for every semantic and durable decision.

#![cfg(target_os = "linux")]

#[path = "worker_linux_verifier.rs"]
pub(crate) mod worker_linux_verifier;
#[path = "worker_local_linux_auth.rs"]
mod worker_local_linux_auth;
#[path = "worker_local_linux_connection.rs"]
mod worker_local_linux_connection;
#[path = "worker_local_linux_fs.rs"]
mod worker_local_linux_fs;
#[path = "worker_local_linux_image.rs"]
mod worker_local_linux_image;
#[cfg(test)]
#[path = "worker_local_linux_image_tests.rs"]
mod worker_local_linux_image_tests;
#[path = "worker_local_linux_process.rs"]
mod worker_local_linux_process;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use tokio::net::UnixListener;
use tokio::time::{Instant, timeout_at};

use crate::worker_frame_io::{ConnectionDeadline, FrameIoError, WorkerFrameIo};
use worker_linux_verifier::{VerifierController, VerifierFailure, VerifierOutcome};
use worker_local_linux_auth::authenticate_credential;
use worker_local_linux_connection::ExchangeGuard;
pub use worker_local_linux_connection::{AuthenticatedWorkerConnection, LinuxPeerWitness};
use worker_local_linux_fs::{HeldEndpoint, is_canonical_absolute};
use worker_local_linux_image::HeldImage;
use worker_local_linux_process::ProtectedCredential;

/// The fixed authentication profile prefix, including its NUL terminator.
pub const AUTH_MAGIC: &[u8; 25] = b"ascension-worker-auth-v1\0";
/// Maximum private credential bytes accepted after [`AUTH_MAGIC`].
pub const MAX_CREDENTIAL_BYTES: usize = 4_096;
/// Maximum auth body after the four-byte big-endian body length.
pub const MAX_AUTH_BODY_BYTES: usize = AUTH_MAGIC.len() + MAX_CREDENTIAL_BYTES;

/// Run the fixed peer-verifier child over its inherited control socket.
/// This entry point supplies no admission authority and accepts no paths or commands.
/// The executable must select it before loading ordinary runtime configuration.
pub fn run_peer_verifier_from_stdin() -> Result<(), LinuxTransportError> {
    worker_linux_verifier::run_verifier_from_stdin()
}

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
    verifier: VerifierController,
}

impl LinuxWorkerListener {
    fn bind(config: LinuxWorkerConfig) -> Result<Self, LinuxTransportError> {
        let owner_uid = rustix::process::getuid().as_raw();
        let credential = ProtectedCredential::open(&config.credential, owner_uid)?;
        let image = HeldImage::open(
            &config.peer.executable,
            owner_uid,
            &config.peer.executable_sha256,
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
            verifier: VerifierController::new().map_err(|_| LinuxTransportError::Io)?,
        })
    }

    /// Accepts one connection and authenticates it before exposing any frame
    /// bytes. `deadline` must be created before this call; it is reused for
    /// accept, peer proof, auth prelude and JSON framing. Peer proof contains
    /// bounded synchronous kernel/filesystem calls. They are checked before
    /// and after each phase, but cannot be preempted while the kernel is
    /// servicing a syscall; this adapter does not detach a blocking worker.
    pub async fn accept_authenticated(
        &self,
        deadline: ConnectionDeadline,
    ) -> Result<AuthenticatedWorkerConnection, LinuxTransportError> {
        let exchange = ExchangeGuard::acquire(&self.active)?;
        let instant = deadline.instant();
        let endpoint = self.endpoint.duplicate_verifier_path()?;
        self.verifier
            .verify_endpoint(endpoint, instant)
            .await
            .map_err(|error| match error {
                VerifierFailure::Configuration => LinuxTransportError::Configuration,
                VerifierFailure::Deadline => LinuxTransportError::Deadline,
                VerifierFailure::Busy => LinuxTransportError::Busy,
                VerifierFailure::Poisoned | VerifierFailure::Io => LinuxTransportError::Io,
            })?;
        let (stream, _) = timeout_at(instant, self.listener.accept())
            .await
            .map_err(|_| LinuxTransportError::Deadline)?
            .map_err(|_| LinuxTransportError::Io)?;
        if Instant::now() >= instant {
            return Err(LinuxTransportError::Deadline);
        }
        let endpoint = self.endpoint.duplicate_verifier_path()?;
        let verifier = self
            .verifier
            .verify(&stream, &self.peer, &self.image, endpoint, instant)
            .await
            .map_err(|error| match error {
                VerifierFailure::Configuration => LinuxTransportError::Configuration,
                VerifierFailure::Busy => LinuxTransportError::Busy,
                VerifierFailure::Deadline => LinuxTransportError::Deadline,
                VerifierFailure::Poisoned | VerifierFailure::Io => LinuxTransportError::Io,
            })?;
        let witness = match verifier {
            VerifierOutcome::Accepted { pidfd, image_fd } => LinuxPeerWitness::from_verified(
                pidfd,
                self.peer.pid,
                self.peer.uid,
                HeldImage::from_verified_fd(
                    image_fd,
                    self.peer.uid,
                    self.image.identity(),
                    &self.peer.executable_sha256,
                )
                .map_err(|_| LinuxTransportError::Io)?,
            ),
            VerifierOutcome::Rejected => return Err(LinuxTransportError::Peer),
            VerifierOutcome::Closed => return Err(LinuxTransportError::Closed),
        };
        let mut stream = stream;
        authenticate_credential(&mut stream, &self.credential, instant).await?;
        if Instant::now() >= instant {
            return Err(LinuxTransportError::Deadline);
        }
        Ok(AuthenticatedWorkerConnection::new(
            WorkerFrameIo::new(stream, deadline),
            witness,
            exchange,
        ))
    }
}
