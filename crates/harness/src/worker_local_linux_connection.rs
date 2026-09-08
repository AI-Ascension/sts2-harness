// SPDX-License-Identifier: MIT

//! Authenticated connection and single-exchange lifecycle types.

#![cfg(target_os = "linux")]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rustix::fd::OwnedFd;

use super::worker_local_linux_image::HeldImage;
use super::worker_local_linux_stream::CredentialStream;
use super::{LinuxTransportError, LinuxWorkerListener};
use crate::worker_frame_io::WorkerFrameIo;
use crate::worker_handoff::MAX_FRAME_BYTES;

/// Opaque, non-serializable proof of the connected peer's authenticated identity.
pub struct LinuxPeerWitness {
    pub(super) _pidfd: OwnedFd,
    pub(super) _pid: u32,
    pub(super) _uid: u32,
    pub(super) _image: HeldImage,
}

impl LinuxPeerWitness {
    pub(super) fn from_verified(pidfd: OwnedFd, pid: u32, uid: u32, image: HeldImage) -> Self {
        Self {
            _pidfd: pidfd,
            _pid: pid,
            _uid: uid,
            _image: image,
        }
    }
}

/// One authenticated connection. It owns the stream and held peer fd;
/// dropping it closes both. It permits exactly one request followed by one
/// response and cannot resume a failed or cancelled operation.
pub struct AuthenticatedWorkerConnection<'a> {
    io: WorkerFrameIo<CredentialStream>,
    witness: LinuxPeerWitness,
    _exchange: ExchangeGuard,
    request_read: bool,
    response_written: bool,
    listener: &'a LinuxWorkerListener,
    peer_stream: OwnedFd,
    deadline: tokio::time::Instant,
}

impl<'a> AuthenticatedWorkerConnection<'a> {
    pub(super) fn new(
        io: WorkerFrameIo<CredentialStream>,
        witness: LinuxPeerWitness,
        exchange: ExchangeGuard,
        listener: &'a LinuxWorkerListener,
        peer_stream: OwnedFd,
        deadline: tokio::time::Instant,
    ) -> Self {
        Self {
            io,
            witness,
            _exchange: exchange,
            request_read: false,
            response_written: false,
            listener,
            peer_stream,
            deadline,
        }
    }

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
        self.request_read = true;
        // Close before awaiting: cancellation or a partial/failed frame must
        // never leave a reusable exchange behind.
        self.response_written = true;
        let request = self.io.read_frame(MAX_FRAME_BYTES).await?;
        // A live PID can exec a different image during framing. Recheck the
        // original connection's native peer and endpoint after the complete
        // request, under the same deadline, before exposing admission bytes.
        self.witness = self
            .listener
            .verify_peer(&self.peer_stream, self.deadline)
            .await?;
        self.response_written = false;
        Ok(request)
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
        self.response_written = true;
        self.io
            .write_frame(response, MAX_FRAME_BYTES)
            .await
            .map_err(Into::into)
    }
}

pub struct ExchangeGuard(Arc<AtomicBool>);

impl ExchangeGuard {
    pub(crate) fn acquire(active: &Arc<AtomicBool>) -> Result<Self, LinuxTransportError> {
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
