// SPDX-License-Identifier: MIT

//! Descriptor I/O for the synchronous verifier helper.

#![cfg(target_os = "linux")]

use std::ffi::OsString;
use std::io::{IoSlice, IoSliceMut};
use std::mem::MaybeUninit;
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::ffi::OsStringExt;

use rustix::event::{PollFd, PollFlags, poll};
use rustix::io::Errno;
use rustix::net::{
    RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags, SendAncillaryBuffer,
    SendAncillaryMessage, SendFlags, recvmsg, sendmsg,
};

use super::super::worker_local_linux_fs::FileIdentity;
use super::super::worker_local_linux_image::HeldImage;
use super::super::{LinuxPeerIdentity, LinuxTransportError};
use super::helper::VerifierRequest;
use super::protocol::{
    CONTROL_POLL_GRACE, CONTROL_VERSION, ENDPOINT_REQUEST_FD_COUNT, MAX_CONTROL_PACKET_BYTES,
    NONCE_BYTES, REQUEST_ENDPOINT_KIND, REQUEST_FD_COUNT, REQUEST_KIND, RESPONSE_FD_COUNT,
    ResponsePacket, read_u16, read_u32, read_u64,
};

pub(super) fn receive_request(
    control: &OwnedFd,
) -> Result<Option<VerifierRequest>, LinuxTransportError> {
    let mut bytes = [0_u8; MAX_CONTROL_PACKET_BYTES];
    let mut iov = [IoSliceMut::new(&mut bytes)];
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(REQUEST_FD_COUNT))];
    let mut ancillary = RecvAncillaryBuffer::new(&mut space);
    let message = loop {
        match recvmsg(control, &mut iov, &mut ancillary, RecvFlags::CMSG_CLOEXEC) {
            Ok(message) => break message,
            Err(error) if error == Errno::AGAIN => {
                let mut descriptor = PollFd::new(
                    control,
                    PollFlags::IN | PollFlags::ERR | PollFlags::HUP | PollFlags::NVAL,
                );
                poll(std::slice::from_mut(&mut descriptor), None)
                    .map_err(|_| LinuxTransportError::Io)?;
            }
            Err(_) => return Err(LinuxTransportError::Io),
        }
    };
    let mut fds = Vec::new();
    for message in ancillary.drain() {
        if let RecvAncillaryMessage::ScmRights(rights) = message {
            fds.extend(rights);
        }
    }
    if message.bytes == 0 {
        return Ok(None);
    }
    if message.bytes > MAX_CONTROL_PACKET_BYTES
        || message
            .flags
            .intersects(ReturnFlags::TRUNC | ReturnFlags::CTRUNC)
    {
        return Err(LinuxTransportError::Io);
    }
    parse_request(&bytes[..message.bytes], fds).map(Some)
}

fn parse_request(bytes: &[u8], fds: Vec<OwnedFd>) -> Result<VerifierRequest, LinuxTransportError> {
    match bytes.get(1).copied() {
        Some(REQUEST_KIND) => parse_peer_request(bytes, fds),
        Some(REQUEST_ENDPOINT_KIND) => parse_endpoint_request(bytes, fds),
        _ => Err(LinuxTransportError::Io),
    }
}

fn parse_peer_request(
    bytes: &[u8],
    mut fds: Vec<OwnedFd>,
) -> Result<VerifierRequest, LinuxTransportError> {
    if bytes.len()
        < super::protocol::CONTROL_HEADER_BYTES
            + NONCE_BYTES
            + 4
            + 4
            + 8
            + super::protocol::DIGEST_BYTES
            + 8
            + 8
            + 8
            + 8
            + 4
            + 2
        || bytes[0] != CONTROL_VERSION
        || bytes[1] != REQUEST_KIND
        || usize::from(bytes[2]) != REQUEST_FD_COUNT
        || bytes[3] != 0
        || fds.len() != REQUEST_FD_COUNT
    {
        return Err(LinuxTransportError::Io);
    }
    let mut offset = super::protocol::CONTROL_HEADER_BYTES;
    let mut nonce = [0_u8; NONCE_BYTES];
    nonce.copy_from_slice(&bytes[offset..offset + NONCE_BYTES]);
    offset += NONCE_BYTES;
    let uid = read_u32(bytes, &mut offset)?;
    let pid = read_u32(bytes, &mut offset)?;
    let start_token = read_u64(bytes, &mut offset)?;
    let mut digest = [0_u8; super::protocol::DIGEST_BYTES];
    digest.copy_from_slice(
        bytes
            .get(offset..offset + super::protocol::DIGEST_BYTES)
            .ok_or(LinuxTransportError::Io)?,
    );
    offset += super::protocol::DIGEST_BYTES;
    let device = read_u64(bytes, &mut offset)?;
    let inode = read_u64(bytes, &mut offset)?;
    let endpoint_device = read_u64(bytes, &mut offset)?;
    let endpoint_inode = read_u64(bytes, &mut offset)?;
    let deadline_ms = read_u32(bytes, &mut offset)?;
    if !(1..=5_000).contains(&deadline_ms) {
        return Err(LinuxTransportError::Io);
    }
    let path_len = usize::from(read_u16(bytes, &mut offset)?);
    if path_len == 0
        || path_len > super::protocol::MAX_PATH_BYTES
        || bytes.len() < offset + path_len + 2
    {
        return Err(LinuxTransportError::Io);
    }
    let path_end = offset
        .checked_add(path_len)
        .ok_or(LinuxTransportError::Io)?;
    let executable_path =
        std::path::PathBuf::from(OsString::from_vec(bytes[offset..path_end].to_vec()));
    offset = path_end;
    let endpoint_len = usize::from(read_u16(bytes, &mut offset)?);
    if endpoint_len == 0
        || endpoint_len > super::protocol::MAX_PATH_BYTES
        || bytes.len() != offset + endpoint_len
    {
        return Err(LinuxTransportError::Io);
    }
    let endpoint_leaf = OsString::from_vec(bytes[offset..].to_vec());
    let endpoint_parent = fds.pop().ok_or(LinuxTransportError::Io)?;
    let image_fd = fds.pop().ok_or(LinuxTransportError::Io)?;
    let stream = fds.pop().ok_or(LinuxTransportError::Io)?;
    let image = HeldImage::from_verified_fd(
        image_fd,
        rustix::process::getuid().as_raw(),
        FileIdentity::from_parts(device, inode),
        &digest,
    )
    .map_err(|_| LinuxTransportError::Io)?;
    Ok(VerifierRequest::Peer {
        nonce,
        expected: LinuxPeerIdentity::new(uid, pid, start_token, executable_path, digest)
            .map_err(|_| LinuxTransportError::Io)?,
        approved_identity: FileIdentity::from_parts(device, inode),
        endpoint_identity: FileIdentity::from_parts(endpoint_device, endpoint_inode),
        endpoint_leaf,
        deadline_ms,
        stream,
        image,
        endpoint_parent,
    })
}

fn parse_endpoint_request(
    bytes: &[u8],
    mut fds: Vec<OwnedFd>,
) -> Result<VerifierRequest, LinuxTransportError> {
    if bytes.len() < super::protocol::CONTROL_HEADER_BYTES + NONCE_BYTES + 8 + 8 + 4 + 2
        || bytes[0] != CONTROL_VERSION
        || bytes[1] != REQUEST_ENDPOINT_KIND
        || usize::from(bytes[2]) != ENDPOINT_REQUEST_FD_COUNT
        || bytes[3] != 0
        || fds.len() != ENDPOINT_REQUEST_FD_COUNT
    {
        return Err(LinuxTransportError::Io);
    }
    let mut offset = super::protocol::CONTROL_HEADER_BYTES;
    let mut nonce = [0_u8; NONCE_BYTES];
    nonce.copy_from_slice(&bytes[offset..offset + NONCE_BYTES]);
    offset += NONCE_BYTES;
    let endpoint_device = read_u64(bytes, &mut offset)?;
    let endpoint_inode = read_u64(bytes, &mut offset)?;
    let deadline_ms = read_u32(bytes, &mut offset)?;
    if !(1..=5_000).contains(&deadline_ms) {
        return Err(LinuxTransportError::Io);
    }
    let endpoint_len = usize::from(read_u16(bytes, &mut offset)?);
    if endpoint_len == 0
        || endpoint_len > super::protocol::MAX_PATH_BYTES
        || bytes.len() != offset + endpoint_len
    {
        return Err(LinuxTransportError::Io);
    }
    let endpoint_leaf = OsString::from_vec(bytes[offset..].to_vec());
    let endpoint_parent = fds.pop().ok_or(LinuxTransportError::Io)?;
    Ok(VerifierRequest::Endpoint {
        nonce,
        endpoint_identity: FileIdentity::from_parts(endpoint_device, endpoint_inode),
        endpoint_leaf,
        deadline_ms,
        endpoint_parent,
    })
}

pub(super) fn send_response(
    control: &OwnedFd,
    response: ResponsePacket,
) -> Result<(), LinuxTransportError> {
    let borrowed = response.fds.iter().map(AsFd::as_fd).collect::<Vec<_>>();
    if borrowed.len() != usize::from(response.bytes[2]) {
        return Err(LinuxTransportError::Io);
    }
    loop {
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(RESPONSE_FD_COUNT))];
        let mut ancillary = SendAncillaryBuffer::new(&mut space);
        if !borrowed.is_empty() && !ancillary.push(SendAncillaryMessage::ScmRights(&borrowed)) {
            return Err(LinuxTransportError::Io);
        }
        match sendmsg(
            control,
            &[IoSlice::new(&response.bytes)],
            &mut ancillary,
            SendFlags::NOSIGNAL,
        ) {
            Ok(written) if written == response.bytes.len() => return Ok(()),
            Ok(_) => return Err(LinuxTransportError::Io),
            Err(error) if error == Errno::AGAIN => {
                let mut descriptor = PollFd::new(
                    control,
                    PollFlags::OUT | PollFlags::ERR | PollFlags::HUP | PollFlags::NVAL,
                );
                poll(
                    std::slice::from_mut(&mut descriptor),
                    Some(&CONTROL_POLL_GRACE),
                )
                .map_err(|_| LinuxTransportError::Io)?;
                let events = descriptor.revents();
                if events.intersects(PollFlags::ERR | PollFlags::HUP | PollFlags::NVAL) {
                    return Err(LinuxTransportError::Io);
                }
            }
            Err(_) => return Err(LinuxTransportError::Io),
        }
    }
}
