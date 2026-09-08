// SPDX-License-Identifier: MIT

//! Authenticated connection and single-exchange lifecycle types.

#![cfg(target_os = "linux")]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rustix::fd::OwnedFd;
use tokio::net::UnixStream;

use super::LinuxTransportError;
use super::worker_local_linux_image::HeldImage;
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
pub struct AuthenticatedWorkerConnection {
    io: WorkerFrameIo<UnixStream>,
    witness: LinuxPeerWitness,
    _exchange: ExchangeGuard,
    request_read: bool,
    response_written: bool,
}

impl AuthenticatedWorkerConnection {
    pub(super) fn new(
        io: WorkerFrameIo<UnixStream>,
        witness: LinuxPeerWitness,
        exchange: ExchangeGuard,
    ) -> Self {
        Self {
            io,
            witness,
            _exchange: exchange,
            request_read: false,
            response_written: false,
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
