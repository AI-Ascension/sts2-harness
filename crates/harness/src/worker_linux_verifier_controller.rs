// SPDX-License-Identifier: MIT

//! Parent-side verifier lease and request controller.

#![cfg(target_os = "linux")]

use std::os::fd::{AsFd, OwnedFd};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use rustix::io::dup;
use rustix::net::{AddressFamily, SocketFlags, SocketType, socketpair};
use tokio::io::unix::AsyncFd;
use tokio::net::UnixStream;
use tokio::time::{Instant, timeout_at};

use super::super::LinuxPeerIdentity;
use super::super::worker_local_linux_fs::FileIdentity;
use super::super::worker_local_linux_image::HeldImage;
use super::lifecycle::{
    ChildStatus, SESSION_ACTIVE, SESSION_AVAILABLE, SESSION_POISONED, VerifierSession,
};
use super::protocol::{
    VerifierFailure, VerifierOutcome, decode_endpoint_response, decode_response,
    encode_endpoint_request, encode_request,
};
use super::transport::{receive_packet, send_packet};

/// One fixed verifier process and its private control channel. The process is
/// reused serially, so an accepted listener has at most one verifier
/// descendant and never creates a process per connection.
pub struct VerifierController {
    session: Arc<VerifierSession>,
}

impl VerifierController {
    /// Starts the one fixed helper while the listener is being bound. This
    /// keeps helper process startup outside each connection's five-second
    /// identity budget; the control descriptor is registered with Tokio only
    /// when the first async verification call runs.
    pub(crate) fn new() -> Result<Self, VerifierFailure> {
        if !super::lifecycle::may_launch_helper() {
            return Err(VerifierFailure::Poisoned);
        }
        let (parent, child_control) = socketpair(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
            None,
        )
        .map_err(|_| VerifierFailure::Io)?;
        let child = spawn_fixed_verifier(child_control)?;
        Ok(Self {
            session: Arc::new(VerifierSession {
                control: std::sync::Mutex::new(Some(parent)),
                async_control: std::sync::Mutex::new(None),
                child: std::sync::Mutex::new(child),
                state: std::sync::atomic::AtomicU8::new(SESSION_AVAILABLE),
                cleanup: std::sync::atomic::AtomicU8::new(0),
            }),
        })
    }

    pub(crate) async fn verify(
        &self,
        stream: &UnixStream,
        expected: &LinuxPeerIdentity,
        approved_image: &HeldImage,
        endpoint: (OwnedFd, FileIdentity, Vec<u8>),
        deadline: Instant,
    ) -> Result<VerifierOutcome, VerifierFailure> {
        let session = self.ensure_session()?;
        let lease = session.acquire()?;
        session.ensure_async_control()?;
        let stream_fd = dup(stream.as_fd()).map_err(|_| VerifierFailure::Io)?;
        let image_fd = approved_image
            .duplicate_fd()
            .map_err(|_| VerifierFailure::Io)?;
        let (request, nonce) =
            encode_request(expected, approved_image, endpoint.1, &endpoint.2, deadline)?;
        let response = timeout_at(
            deadline,
            session.exchange(&request, [&stream_fd, &image_fd, &endpoint.0]),
        )
        .await;
        let response = match response {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                lease.poison();
                return Err(error);
            }
            Err(_) => {
                lease.poison();
                return Err(VerifierFailure::Deadline);
            }
        };
        let outcome = match decode_response(&response.bytes, &nonce, response.fds) {
            Ok(outcome) => outcome,
            Err(error) => {
                lease.poison();
                return Err(error);
            }
        };
        if session.child_status() != ChildStatus::Alive {
            lease.poison();
            return Err(VerifierFailure::Poisoned);
        }
        lease.complete();
        Ok(outcome)
    }

    pub(crate) async fn verify_endpoint(
        &self,
        endpoint: (OwnedFd, FileIdentity, Vec<u8>),
        deadline: Instant,
    ) -> Result<(), VerifierFailure> {
        let session = self.ensure_session()?;
        let lease = session.acquire()?;
        session.ensure_async_control()?;
        let (request, nonce) = encode_endpoint_request(endpoint.1, &endpoint.2, deadline)?;
        let response = timeout_at(deadline, session.exchange(&request, [&endpoint.0])).await;
        let response = match response {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                lease.poison();
                return Err(error);
            }
            Err(_) => {
                lease.poison();
                return Err(VerifierFailure::Deadline);
            }
        };
        if let Err(error) = decode_endpoint_response(&response.bytes, &nonce, response.fds) {
            lease.poison();
            return Err(error);
        }
        if session.child_status() != ChildStatus::Alive {
            lease.poison();
            return Err(VerifierFailure::Poisoned);
        }
        lease.complete();
        Ok(())
    }

    fn ensure_session(&self) -> Result<Arc<VerifierSession>, VerifierFailure> {
        let session = Arc::clone(&self.session);
        if session.state.load(std::sync::atomic::Ordering::Acquire) == SESSION_POISONED {
            return Err(VerifierFailure::Poisoned);
        }
        Ok(session)
    }
}

impl Drop for VerifierController {
    fn drop(&mut self) {
        self.session
            .state
            .store(SESSION_POISONED, std::sync::atomic::Ordering::Release);
        self.session.poison();
    }
}

impl VerifierSession {
    fn ensure_async_control(&self) -> Result<(), VerifierFailure> {
        let mut async_control = self
            .async_control
            .try_lock()
            .map_err(|_| VerifierFailure::Poisoned)?;
        if async_control.is_some() {
            return Ok(());
        }
        let raw = self
            .control
            .try_lock()
            .map_err(|_| VerifierFailure::Poisoned)?
            .take()
            .ok_or(VerifierFailure::Poisoned)?;
        let control = AsyncFd::new(raw).map_err(|_| VerifierFailure::Io)?;
        async_control.replace(Arc::new(control));
        Ok(())
    }

    fn async_control(&self) -> Result<Arc<AsyncFd<OwnedFd>>, VerifierFailure> {
        self.async_control
            .try_lock()
            .map_err(|_| VerifierFailure::Poisoned)?
            .as_ref()
            .cloned()
            .ok_or(VerifierFailure::Poisoned)
    }

    fn acquire(self: &Arc<Self>) -> Result<VerifierLease, VerifierFailure> {
        self.state
            .compare_exchange(
                SESSION_AVAILABLE,
                SESSION_ACTIVE,
                std::sync::atomic::Ordering::Acquire,
                std::sync::atomic::Ordering::Relaxed,
            )
            .map(|_| VerifierLease {
                session: Arc::clone(self),
                completed: false,
            })
            .map_err(|state| {
                if state == SESSION_POISONED {
                    VerifierFailure::Poisoned
                } else {
                    VerifierFailure::Busy
                }
            })
    }

    async fn exchange<'a, I>(
        &'a self,
        request: &[u8],
        fds: I,
    ) -> Result<super::protocol::VerifierResponse, VerifierFailure>
    where
        I: IntoIterator<Item = &'a OwnedFd>,
    {
        let fds = fds.into_iter().collect::<Vec<_>>();
        if fds.is_empty() || fds.len() > super::protocol::REQUEST_FD_COUNT {
            return Err(VerifierFailure::Io);
        }
        let control = self.async_control()?;
        send_packet(&control, request, &fds).await?;
        receive_packet(&control).await
    }
}

struct VerifierLease {
    session: Arc<VerifierSession>,
    completed: bool,
}

impl VerifierLease {
    fn complete(mut self) {
        self.completed = true;
        self.session
            .state
            .store(SESSION_AVAILABLE, std::sync::atomic::Ordering::Release);
    }

    fn poison(mut self) {
        self.completed = true;
        self.session.poison();
    }
}

impl Drop for VerifierLease {
    fn drop(&mut self) {
        if !self.completed {
            self.session.poison();
        }
    }
}

fn spawn_fixed_verifier(control: OwnedFd) -> Result<Child, VerifierFailure> {
    let executable = std::env::current_exe().map_err(|_| VerifierFailure::Io)?;
    let mut command = Command::new(executable);
    #[cfg(test)]
    command
        .args(["--exact", super::TEST_HELPER_NAME, "--nocapture"])
        .env(super::TEST_HELPER_ENV, "1");
    #[cfg(not(test))]
    command.arg("--worker-peer-verifier-v1");
    command
        .stdin(Stdio::from(std::fs::File::from(control)))
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command.spawn().map_err(|_| VerifierFailure::Io)
}
