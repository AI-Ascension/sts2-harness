// SPDX-License-Identifier: MIT

//! Every received byte is accompanied by kernel credentials for the pinned
//! live peer. Passing a connected stream to another process grants no authority.

use std::io::{self, IoSliceMut};
use std::mem::MaybeUninit;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use rustix::fd::OwnedFd;
use rustix::net::{RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, ReturnFlags, recvmsg};
use tokio::io::{AsyncRead, AsyncWrite, Interest, ReadBuf};
use tokio::net::UnixStream;

use super::worker_local_linux_connection::LinuxPeerWitness;
use super::worker_local_linux_process::ensure_pidfd_live;
use super::{LinuxPeerIdentity, LinuxTransportError};

pub(super) struct CredentialStream {
    stream: UnixStream,
    pidfd: OwnedFd,
    pid: u32,
    uid: u32,
    gid: u32,
    failed: bool,
}

impl CredentialStream {
    pub(super) fn new(
        stream: UnixStream,
        expected: &LinuxPeerIdentity,
        witness: &LinuxPeerWitness,
    ) -> Result<Self, LinuxTransportError> {
        // The listener enables this before acceptance, so bytes queued before
        // verification also carry credentials. Reassert it on the owned stream.
        rustix::net::sockopt::set_socket_passcred(&stream, true)
            .map_err(|_| LinuxTransportError::Peer)?;
        let pidfd = rustix::io::dup(&witness._pidfd).map_err(|_| LinuxTransportError::Peer)?;
        Ok(Self {
            stream,
            pidfd,
            pid: expected.pid,
            uid: expected.uid,
            gid: expected.gid,
            failed: false,
        })
    }

    fn live(&self) -> io::Result<()> {
        if self.failed || ensure_pidfd_live(&self.pidfd).is_err() {
            return Err(rejected());
        }
        Ok(())
    }

    fn receive(&self, bytes: &mut [u8]) -> io::Result<usize> {
        self.live()?;
        let mut iov = [IoSliceMut::new(bytes)];
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmCredentials(1))];
        let mut ancillary = RecvAncillaryBuffer::new(&mut space);
        let result = recvmsg(
            &self.stream,
            &mut iov,
            &mut ancillary,
            RecvFlags::DONTWAIT | RecvFlags::CMSG_CLOEXEC,
        )
        .map_err(io::Error::from)?;
        let mut credentials = None;
        for message in ancillary.drain() {
            match message {
                RecvAncillaryMessage::ScmCredentials(value) if credentials.is_none() => {
                    credentials = Some(value);
                }
                _ => return Err(rejected()),
            }
        }
        if result
            .flags
            .intersects(ReturnFlags::TRUNC | ReturnFlags::CTRUNC)
        {
            return Err(rejected());
        }
        if result.bytes == 0 {
            return Ok(0);
        }
        let credentials = credentials.ok_or_else(rejected)?;
        if u32::try_from(credentials.pid.as_raw_pid()).ok() != Some(self.pid)
            || credentials.uid.as_raw() != self.uid
            || credentials.gid.as_raw() != self.gid
        {
            return Err(rejected());
        }
        self.live()?;
        Ok(result.bytes)
    }
}

fn rejected() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "worker peer credentials rejected",
    )
}

impl AsyncRead for CredentialStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        output: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if let Err(error) = self.live() {
            return Poll::Ready(Err(error));
        }
        if output.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        loop {
            ready!(self.stream.poll_read_ready(cx))?;
            // The auth prelude passes through this buffer before reaching its
            // zeroizing owner; do not leave a second credential copy on the stack.
            let mut bytes = zeroize::Zeroizing::new([0_u8; 16 * 1024]);
            let limit = bytes.len().min(output.remaining());
            match self
                .stream
                .try_io(Interest::READABLE, || self.receive(&mut bytes[..limit]))
            {
                Ok(count) => {
                    output.put_slice(&bytes[..count]);
                    return Poll::Ready(Ok(()));
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => continue,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                }
                Err(error) => {
                    self.failed = true;
                    return Poll::Ready(Err(error));
                }
            }
        }
    }
}

impl AsyncWrite for CredentialStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        if let Err(error) = self.live() {
            return Poll::Ready(Err(error));
        }
        Pin::new(&mut self.stream).poll_write(cx, bytes)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if let Err(error) = self.live() {
            return Poll::Ready(Err(error));
        }
        Pin::new(&mut self.stream).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}
