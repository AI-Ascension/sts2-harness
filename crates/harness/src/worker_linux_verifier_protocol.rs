// SPDX-License-Identifier: MIT

//! Fixed private wire schema for the Linux peer verifier.

#![cfg(target_os = "linux")]

use std::os::fd::OwnedFd;
use std::os::unix::ffi::OsStrExt;

use tokio::time::Instant;
use uuid::Uuid;

use super::super::worker_local_linux_fs::FileIdentity;
use super::super::worker_local_linux_image::HeldImage;
use super::super::{LinuxPeerIdentity, LinuxTransportError};

/// Private control schema revision. This is independent from the frozen
/// worker handoff/authentication wire.
pub(super) const CONTROL_VERSION: u8 = 2;
pub(super) const REQUEST_KIND: u8 = 1;
pub(super) const RESPONSE_ACCEPTED: u8 = 1;
pub(super) const RESPONSE_REJECTED: u8 = 2;
pub(super) const RESPONSE_CLOSED: u8 = 3;
pub(super) const RESPONSE_FAILURE: u8 = 4;
pub(super) const RESPONSE_CONFIGURATION: u8 = 5;
pub(super) const REQUEST_ENDPOINT_KIND: u8 = 2;
pub(super) const RESPONSE_ENDPOINT_VALID: u8 = 6;
pub(super) const REQUEST_FD_COUNT: usize = 3;
pub(super) const ENDPOINT_REQUEST_FD_COUNT: usize = 1;
pub(super) const RESPONSE_FD_COUNT: usize = 2;
pub(super) const CONTROL_HEADER_BYTES: usize = 4;
pub(super) const NONCE_BYTES: usize = 16;
pub(super) const DIGEST_BYTES: usize = 32;
pub(super) const MAX_PATH_BYTES: usize = 4_096;
pub(super) const MAX_CONTROL_PACKET_BYTES: usize = CONTROL_HEADER_BYTES
    + NONCE_BYTES
    + 4
    + 4
    + 4
    + 8
    + DIGEST_BYTES
    + 8
    + 8
    + 8
    + 8
    + 4
    + 2
    + MAX_PATH_BYTES
    + 2
    + MAX_PATH_BYTES;
pub(super) const RESPONSE_BYTES: usize = CONTROL_HEADER_BYTES + NONCE_BYTES;

/// Poll interval used by the synchronous helper side of the private channel.
pub(super) const CONTROL_POLL_GRACE: rustix::event::Timespec = rustix::event::Timespec {
    tv_sec: 0,
    tv_nsec: 100_000_000,
};

/// Failure classification used by the parent to distinguish an ordinary
/// rejected peer from a verifier/control failure that poisons the lease.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerifierFailure {
    Configuration,
    Busy,
    Deadline,
    Poisoned,
    Io,
}

/// Result of one private verifier request. Accepted results contain only
/// kernel-held descriptors; no process number or path is exposed to callers.
pub enum VerifierOutcome {
    Accepted { pidfd: OwnedFd, image_fd: OwnedFd },
    Rejected,
    Closed,
}

pub(super) struct VerifierResponse {
    pub(super) bytes: Vec<u8>,
    pub(super) fds: Vec<OwnedFd>,
}

pub(super) struct ResponsePacket {
    pub(super) bytes: Vec<u8>,
    pub(super) fds: Vec<OwnedFd>,
}

impl ResponsePacket {
    pub(super) fn without_fds(kind: u8, nonce: [u8; NONCE_BYTES]) -> Self {
        Self::with_fds(kind, nonce, Vec::new())
    }

    pub(super) fn with_fds(kind: u8, nonce: [u8; NONCE_BYTES], fds: Vec<OwnedFd>) -> Self {
        let mut bytes = Vec::with_capacity(RESPONSE_BYTES);
        bytes.extend_from_slice(&[
            CONTROL_VERSION,
            kind,
            u8::try_from(fds.len()).unwrap_or(0),
            0,
        ]);
        bytes.extend_from_slice(&nonce);
        Self { bytes, fds }
    }
}

pub(super) fn encode_request(
    expected: &LinuxPeerIdentity,
    approved_image: &HeldImage,
    endpoint_identity: FileIdentity,
    endpoint_leaf: &[u8],
    deadline: Instant,
) -> Result<(Vec<u8>, [u8; NONCE_BYTES]), VerifierFailure> {
    let path = expected.executable.as_os_str().as_bytes();
    if path.is_empty()
        || path.len() > MAX_PATH_BYTES
        || endpoint_leaf.is_empty()
        || endpoint_leaf.len() > MAX_PATH_BYTES
    {
        return Err(VerifierFailure::Io);
    }
    let remaining = deadline.saturating_duration_since(Instant::now());
    let deadline_ms = u32::try_from(remaining.as_millis())
        .ok()
        .filter(|value| *value > 0 && *value <= 5_000)
        .ok_or(VerifierFailure::Deadline)?;
    let (device, inode) = approved_image.identity_parts();
    let nonce = Uuid::new_v4().into_bytes();
    let mut packet = Vec::with_capacity(MAX_CONTROL_PACKET_BYTES);
    packet.extend_from_slice(&[
        CONTROL_VERSION,
        REQUEST_KIND,
        u8::try_from(REQUEST_FD_COUNT).map_err(|_| VerifierFailure::Io)?,
        0,
    ]);
    packet.extend_from_slice(&nonce);
    packet.extend_from_slice(&expected.uid.to_be_bytes());
    packet.extend_from_slice(&expected.gid.to_be_bytes());
    packet.extend_from_slice(&expected.pid.to_be_bytes());
    packet.extend_from_slice(&expected.start_token.to_be_bytes());
    packet.extend_from_slice(&expected.executable_sha256);
    packet.extend_from_slice(&device.to_be_bytes());
    packet.extend_from_slice(&inode.to_be_bytes());
    let (endpoint_device, endpoint_inode) = endpoint_identity.parts();
    packet.extend_from_slice(&endpoint_device.to_be_bytes());
    packet.extend_from_slice(&endpoint_inode.to_be_bytes());
    packet.extend_from_slice(&deadline_ms.to_be_bytes());
    packet.extend_from_slice(
        &u16::try_from(path.len())
            .map_err(|_| VerifierFailure::Io)?
            .to_be_bytes(),
    );
    packet.extend_from_slice(path);
    packet.extend_from_slice(
        &u16::try_from(endpoint_leaf.len())
            .map_err(|_| VerifierFailure::Io)?
            .to_be_bytes(),
    );
    packet.extend_from_slice(endpoint_leaf);
    if packet.len() > MAX_CONTROL_PACKET_BYTES {
        return Err(VerifierFailure::Io);
    }
    Ok((packet, nonce))
}

pub(super) fn encode_endpoint_request(
    endpoint_identity: FileIdentity,
    endpoint_leaf: &[u8],
    deadline: Instant,
) -> Result<(Vec<u8>, [u8; NONCE_BYTES]), VerifierFailure> {
    if endpoint_leaf.is_empty() || endpoint_leaf.len() > MAX_PATH_BYTES {
        return Err(VerifierFailure::Io);
    }
    let remaining = deadline.saturating_duration_since(Instant::now());
    let deadline_ms = u32::try_from(remaining.as_millis())
        .ok()
        .filter(|value| *value > 0 && *value <= 5_000)
        .ok_or(VerifierFailure::Deadline)?;
    let nonce = Uuid::new_v4().into_bytes();
    let (device, inode) = endpoint_identity.parts();
    let mut packet = Vec::with_capacity(
        CONTROL_HEADER_BYTES + NONCE_BYTES + 8 + 8 + 4 + 2 + endpoint_leaf.len(),
    );
    packet.extend_from_slice(&[
        CONTROL_VERSION,
        REQUEST_ENDPOINT_KIND,
        u8::try_from(ENDPOINT_REQUEST_FD_COUNT).map_err(|_| VerifierFailure::Io)?,
        0,
    ]);
    packet.extend_from_slice(&nonce);
    packet.extend_from_slice(&device.to_be_bytes());
    packet.extend_from_slice(&inode.to_be_bytes());
    packet.extend_from_slice(&deadline_ms.to_be_bytes());
    packet.extend_from_slice(
        &u16::try_from(endpoint_leaf.len())
            .map_err(|_| VerifierFailure::Io)?
            .to_be_bytes(),
    );
    packet.extend_from_slice(endpoint_leaf);
    if packet.len() > MAX_CONTROL_PACKET_BYTES {
        return Err(VerifierFailure::Io);
    }
    Ok((packet, nonce))
}

pub(super) fn decode_response(
    bytes: &[u8],
    expected_nonce: &[u8; NONCE_BYTES],
    mut fds: Vec<OwnedFd>,
) -> Result<VerifierOutcome, VerifierFailure> {
    if bytes.len() != RESPONSE_BYTES
        || bytes[0] != CONTROL_VERSION
        || bytes[3] != 0
        || &bytes[CONTROL_HEADER_BYTES..] != expected_nonce
    {
        return Err(VerifierFailure::Io);
    }
    let fd_count = usize::from(bytes[2]);
    match (bytes[1], fd_count) {
        (RESPONSE_ACCEPTED, RESPONSE_FD_COUNT) if fds.len() == RESPONSE_FD_COUNT => {
            let image_fd = fds.pop().ok_or(VerifierFailure::Io)?;
            let pidfd = fds.pop().ok_or(VerifierFailure::Io)?;
            Ok(VerifierOutcome::Accepted { pidfd, image_fd })
        }
        (RESPONSE_REJECTED, 0) if fds.is_empty() => Ok(VerifierOutcome::Rejected),
        (RESPONSE_CLOSED, 0) if fds.is_empty() => Ok(VerifierOutcome::Closed),
        (RESPONSE_FAILURE, 0) if fds.is_empty() => Err(VerifierFailure::Poisoned),
        (RESPONSE_CONFIGURATION, 0) if fds.is_empty() => Err(VerifierFailure::Configuration),
        _ => Err(VerifierFailure::Io),
    }
}

pub(super) fn decode_endpoint_response(
    bytes: &[u8],
    expected_nonce: &[u8; NONCE_BYTES],
    fds: Vec<OwnedFd>,
) -> Result<(), VerifierFailure> {
    if bytes.len() != RESPONSE_BYTES
        || bytes[0] != CONTROL_VERSION
        || bytes[3] != 0
        || &bytes[CONTROL_HEADER_BYTES..] != expected_nonce
        || !fds.is_empty()
    {
        return Err(VerifierFailure::Io);
    }
    match bytes[1] {
        RESPONSE_ENDPOINT_VALID => Ok(()),
        RESPONSE_CONFIGURATION => Err(VerifierFailure::Configuration),
        RESPONSE_FAILURE => Err(VerifierFailure::Poisoned),
        _ => Err(VerifierFailure::Io),
    }
}

pub(super) fn read_u16(bytes: &[u8], offset: &mut usize) -> Result<u16, LinuxTransportError> {
    let end = offset.checked_add(2).ok_or(LinuxTransportError::Io)?;
    let value = u16::from_be_bytes(
        bytes
            .get(*offset..end)
            .ok_or(LinuxTransportError::Io)?
            .try_into()
            .map_err(|_| LinuxTransportError::Io)?,
    );
    *offset = end;
    Ok(value)
}

pub(super) fn read_u32(bytes: &[u8], offset: &mut usize) -> Result<u32, LinuxTransportError> {
    let end = offset.checked_add(4).ok_or(LinuxTransportError::Io)?;
    let value = u32::from_be_bytes(
        bytes
            .get(*offset..end)
            .ok_or(LinuxTransportError::Io)?
            .try_into()
            .map_err(|_| LinuxTransportError::Io)?,
    );
    *offset = end;
    Ok(value)
}

pub(super) fn read_u64(bytes: &[u8], offset: &mut usize) -> Result<u64, LinuxTransportError> {
    let end = offset.checked_add(8).ok_or(LinuxTransportError::Io)?;
    let value = u64::from_be_bytes(
        bytes
            .get(*offset..end)
            .ok_or(LinuxTransportError::Io)?
            .try_into()
            .map_err(|_| LinuxTransportError::Io)?,
    );
    *offset = end;
    Ok(value)
}
