// SPDX-License-Identifier: MIT

//! Synchronous proof policy for the fixed verifier subprocess.

#![cfg(target_os = "linux")]

use std::ffi::OsString;
use std::os::fd::{AsFd, OwnedFd};
use std::time::Duration;

use rustix::event::{PollFd, PollFlags, poll};
use tokio::time::Instant;

use super::super::worker_local_linux_fs::{FileIdentity, compare_path_identity};
use super::super::worker_local_linux_image::HeldImage;
use super::super::worker_local_linux_process::{VerifiedPeer, verify_peer_fd};
use super::super::{LinuxPeerIdentity, LinuxTransportError};
use super::helper_io::{receive_request, send_response};
use super::protocol::{
    CONTROL_POLL_GRACE, NONCE_BYTES, RESPONSE_ACCEPTED, RESPONSE_CLOSED, RESPONSE_CONFIGURATION,
    RESPONSE_ENDPOINT_VALID, RESPONSE_FAILURE, RESPONSE_REJECTED, ResponsePacket,
};

pub(super) fn run_verifier_from_stdin() -> Result<(), LinuxTransportError> {
    let stdin = std::io::stdin();
    let fd = rustix::io::dup(stdin.as_fd()).map_err(|_| LinuxTransportError::Io)?;
    run_verifier_loop(fd)
}

fn run_verifier_loop(control: OwnedFd) -> Result<(), LinuxTransportError> {
    loop {
        let mut descriptor = PollFd::new(
            &control,
            PollFlags::IN | PollFlags::ERR | PollFlags::HUP | PollFlags::NVAL,
        );
        poll(
            std::slice::from_mut(&mut descriptor),
            Some(&CONTROL_POLL_GRACE),
        )
        .map_err(|_| LinuxTransportError::Io)?;
        let events = descriptor.revents();
        if events.intersects(PollFlags::ERR | PollFlags::NVAL)
            || (events.contains(PollFlags::HUP) && !events.contains(PollFlags::IN))
        {
            return Ok(());
        }
        if !events.contains(PollFlags::IN) {
            continue;
        }
        let request = receive_request(&control)?;
        let Some(request) = request else {
            return Ok(());
        };
        send_response(&control, handle_request(request))?;
    }
}

pub(super) enum VerifierRequest {
    Peer {
        nonce: [u8; NONCE_BYTES],
        expected: LinuxPeerIdentity,
        approved_identity: FileIdentity,
        endpoint_identity: FileIdentity,
        endpoint_leaf: OsString,
        deadline_ms: u32,
        stream: OwnedFd,
        image: HeldImage,
        endpoint_parent: OwnedFd,
    },
    Endpoint {
        nonce: [u8; NONCE_BYTES],
        endpoint_identity: FileIdentity,
        endpoint_leaf: OsString,
        deadline_ms: u32,
        endpoint_parent: OwnedFd,
    },
}

fn handle_request(request: VerifierRequest) -> ResponsePacket {
    match request {
        VerifierRequest::Endpoint {
            nonce,
            endpoint_identity,
            endpoint_leaf,
            deadline_ms,
            endpoint_parent,
        } => {
            let deadline = Instant::now() + Duration::from_millis(u64::from(deadline_ms));
            if Instant::now() >= deadline {
                return ResponsePacket::without_fds(RESPONSE_FAILURE, nonce);
            }
            if compare_path_identity(
                &endpoint_parent,
                &endpoint_leaf,
                endpoint_identity,
                LinuxTransportError::Configuration,
            )
            .is_ok()
            {
                ResponsePacket::without_fds(RESPONSE_ENDPOINT_VALID, nonce)
            } else {
                ResponsePacket::without_fds(RESPONSE_CONFIGURATION, nonce)
            }
        }
        request @ VerifierRequest::Peer { .. } => handle_peer_request(request),
    }
}

fn handle_peer_request(request: VerifierRequest) -> ResponsePacket {
    let VerifierRequest::Peer {
        nonce,
        expected,
        approved_identity,
        endpoint_identity,
        endpoint_leaf,
        deadline_ms,
        stream,
        image,
        endpoint_parent,
    } = request
    else {
        return ResponsePacket::without_fds(RESPONSE_FAILURE, [0; NONCE_BYTES]);
    };
    let deadline = Instant::now() + Duration::from_millis(u64::from(deadline_ms));
    if image.identity() != approved_identity {
        return ResponsePacket::without_fds(RESPONSE_FAILURE, nonce);
    }
    if compare_path_identity(
        &endpoint_parent,
        &endpoint_leaf,
        endpoint_identity,
        LinuxTransportError::Configuration,
    )
    .is_err()
    {
        return ResponsePacket::without_fds(RESPONSE_CONFIGURATION, nonce);
    }
    let response_kind = match verify_peer_fd(stream.as_fd(), &expected, &image, deadline) {
        Ok(VerifiedPeer { pidfd, image }) => {
            let image_fd = match image.duplicate_fd() {
                Ok(fd) => fd,
                Err(_) => return ResponsePacket::without_fds(RESPONSE_FAILURE, nonce),
            };
            if compare_path_identity(
                &endpoint_parent,
                &endpoint_leaf,
                endpoint_identity,
                LinuxTransportError::Configuration,
            )
            .is_err()
            {
                return ResponsePacket::without_fds(RESPONSE_CONFIGURATION, nonce);
            }
            return ResponsePacket::with_fds(RESPONSE_ACCEPTED, nonce, vec![pidfd, image_fd]);
        }
        Err(LinuxTransportError::Deadline) => RESPONSE_FAILURE,
        Err(LinuxTransportError::Closed) => RESPONSE_CLOSED,
        Err(LinuxTransportError::Peer) => RESPONSE_REJECTED,
        Err(_) => RESPONSE_FAILURE,
    };
    ResponsePacket::without_fds(response_kind, nonce)
}
