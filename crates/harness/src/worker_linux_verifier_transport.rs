// SPDX-License-Identifier: MIT

//! Async parent-side transport for the private verifier socketpair.

#![cfg(target_os = "linux")]

use std::io::{IoSlice, IoSliceMut};
use std::mem::MaybeUninit;
use std::os::fd::{AsFd, OwnedFd};

use rustix::net::{
    RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags, SendAncillaryBuffer,
    SendAncillaryMessage, SendFlags, recvmsg, sendmsg,
};
use tokio::io::{Interest, unix::AsyncFd};

use super::protocol::{
    MAX_CONTROL_PACKET_BYTES, REQUEST_FD_COUNT, RESPONSE_BYTES, RESPONSE_FD_COUNT, VerifierFailure,
    VerifierResponse,
};

pub(super) async fn send_packet(
    control: &AsyncFd<OwnedFd>,
    packet: &[u8],
    fds: &[&OwnedFd],
) -> Result<(), VerifierFailure> {
    let result = control
        .async_io(Interest::WRITABLE, |fd| {
            let mut space =
                [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(REQUEST_FD_COUNT))];
            let mut ancillary = SendAncillaryBuffer::new(&mut space);
            let borrowed = fds.iter().map(|fd| fd.as_fd()).collect::<Vec<_>>();
            if !ancillary.push(SendAncillaryMessage::ScmRights(&borrowed)) {
                return Err(std::io::Error::other(
                    "verifier descriptor buffer is too small",
                ));
            }
            let written = sendmsg(
                fd,
                &[IoSlice::new(packet)],
                &mut ancillary,
                SendFlags::NOSIGNAL,
            )?;
            if written != packet.len() {
                return Err(std::io::Error::other("verifier control packet was partial"));
            }
            Ok(())
        })
        .await;
    result.map_err(|_| VerifierFailure::Io)
}

pub(super) async fn receive_packet(
    control: &AsyncFd<OwnedFd>,
) -> Result<VerifierResponse, VerifierFailure> {
    let result = control
        .async_io(Interest::READABLE, |fd| {
            let mut bytes = [0_u8; MAX_CONTROL_PACKET_BYTES];
            let mut iov = [IoSliceMut::new(&mut bytes)];
            let mut space =
                [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(RESPONSE_FD_COUNT))];
            let mut ancillary = RecvAncillaryBuffer::new(&mut space);
            let message = recvmsg(fd, &mut iov, &mut ancillary, RecvFlags::CMSG_CLOEXEC)?;
            let mut fds = Vec::new();
            for message in ancillary.drain() {
                if let RecvAncillaryMessage::ScmRights(rights) = message {
                    fds.extend(rights);
                }
            }
            if message.bytes == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "verifier control closed",
                ));
            }
            if message.bytes > MAX_CONTROL_PACKET_BYTES
                || message
                    .flags
                    .intersects(ReturnFlags::TRUNC | ReturnFlags::CTRUNC)
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "verifier control packet was truncated",
                ));
            }
            Ok((bytes[..message.bytes].to_vec(), fds))
        })
        .await
        .map_err(|_| VerifierFailure::Io)?;
    let (bytes, fds) = result;
    if bytes.len() != RESPONSE_BYTES {
        return Err(VerifierFailure::Io);
    }
    Ok(VerifierResponse { bytes, fds })
}
