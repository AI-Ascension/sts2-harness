// SPDX-License-Identifier: MIT

//! Parent-side verifier lease and request controller.

#![cfg(target_os = "linux")]

use std::os::fd::{AsFd, OwnedFd};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
#[cfg(test)]
use std::sync::{Condvar, Mutex};

use rustix::io::dup;
use rustix::net::{AddressFamily, SocketFlags, SocketType, socketpair};
use tokio::io::unix::AsyncFd;
use tokio::time::{Instant, timeout_at};

use super::super::LinuxPeerIdentity;
use super::super::worker_local_linux_fs::FileIdentity;
use super::super::worker_local_linux_image::HeldImage;
use super::lifecycle::{
    ChildStatus, RegistryError, SESSION_ACTIVE, SESSION_AVAILABLE, SESSION_POISONED,
    SESSION_STARTING, VerifierSession,
};
use super::protocol::{
    VerifierFailure, VerifierOutcome, decode_endpoint_response, decode_response,
    encode_endpoint_request, encode_request,
};
use super::transport::{receive_packet, send_packet};

#[cfg(test)]
static TEST_CONTROLLER_SERIAL: (Mutex<Option<std::thread::ThreadId>>, Condvar) =
    (Mutex::new(None), Condvar::new());

#[cfg(test)]
struct TestControllerSerialGuard {
    owner: Option<std::thread::ThreadId>,
}

#[cfg(test)]
impl TestControllerSerialGuard {
    fn acquire() -> Self {
        let thread = std::thread::current().id();
        let (owners, wake) = &TEST_CONTROLLER_SERIAL;
        let mut owner = owners
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if owner.as_ref() == Some(&thread) {
            // A same-thread constructor is allowed to reach the lifecycle
            // registry and receive its ordinary Occupied error. This avoids
            // deadlocking a test that probes the singleton while holding it.
            return Self { owner: None };
        }
        while owner.is_some() {
            owner = wake
                .wait(owner)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        *owner = Some(thread);
        Self {
            owner: Some(thread),
        }
    }
}

#[cfg(test)]
impl Drop for TestControllerSerialGuard {
    fn drop(&mut self) {
        let Some(thread) = self.owner.take() else {
            return;
        };
        let (owners, wake) = &TEST_CONTROLLER_SERIAL;
        let mut owner = owners
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if owner.as_ref() == Some(&thread) {
            *owner = None;
            wake.notify_one();
        }
    }
}

/// One fixed verifier process and its private control channel. The process is
/// reused serially, so an accepted listener has at most one verifier
/// descendant and never creates a process per connection.
pub struct VerifierController {
    session: Arc<VerifierSession>,
    #[cfg(test)]
    _test_serial: TestControllerSerialGuard,
}

impl VerifierController {
    /// Starts the one fixed helper while the listener is being bound. This
    /// keeps helper process startup outside each connection's five-second
    /// identity budget; the control descriptor is registered with Tokio only
    /// when the first async verification call runs.
    pub(crate) fn new() -> Result<Self, VerifierFailure> {
        // The production registry intentionally owns one helper slot. Keep
        // integration fixtures that compile this module under `cfg(test)`
        // serial, while retaining the same singleton lifecycle contract.
        #[cfg(test)]
        let test_serial = TestControllerSerialGuard::acquire();

        let session = Arc::new(VerifierSession {
            control: std::sync::Mutex::new(None),
            async_control: std::sync::Mutex::new(None),
            child: std::sync::Mutex::new(None),
            state: std::sync::atomic::AtomicU8::new(SESSION_STARTING),
            cleanup: std::sync::atomic::AtomicU8::new(0),
        });
        // Hold the initial guards while the session is globally registered.
        // A concurrent constructor can observe the `Starting` owner but can
        // neither take its handles nor launch around this reservation.
        let mut control = session
            .control
            .try_lock()
            .map_err(|_| VerifierFailure::Poisoned)?;
        let mut child_slot = session
            .child
            .try_lock()
            .map_err(|_| VerifierFailure::Poisoned)?;
        let _launch =
            super::lifecycle::reserve_helper_slot(&session).map_err(map_registry_error)?;
        let (parent, child_control) = match socketpair(
            AddressFamily::UNIX,
            SocketType::SEQPACKET,
            SocketFlags::CLOEXEC | SocketFlags::NONBLOCK,
            None,
        ) {
            Ok(pair) => pair,
            Err(_) => {
                session
                    .state
                    .store(SESSION_POISONED, std::sync::atomic::Ordering::Release);
                drop(child_slot);
                drop(control);
                drop(_launch);
                super::lifecycle::release_reaped_session(&session);
                return Err(VerifierFailure::Io);
            }
        };
        *control = Some(parent);
        let child = match spawn_fixed_verifier(child_control) {
            Ok(child) => child,
            Err(error) => {
                let _ = control.take();
                session
                    .state
                    .store(SESSION_POISONED, std::sync::atomic::Ordering::Release);
                drop(child_slot);
                drop(control);
                drop(_launch);
                super::lifecycle::release_reaped_session(&session);
                return Err(error);
            }
        };
        *child_slot = Some(child);
        session
            .state
            .store(SESSION_AVAILABLE, std::sync::atomic::Ordering::Release);
        drop(child_slot);
        drop(control);
        drop(_launch);
        Ok(Self {
            session,
            #[cfg(test)]
            _test_serial: test_serial,
        })
    }

    pub(crate) async fn verify(
        &self,
        stream: &impl AsFd,
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
        super::lifecycle::release_reaped_session(&self.session);
    }
}

fn map_registry_error(error: RegistryError) -> VerifierFailure {
    match error {
        RegistryError::Busy | RegistryError::Occupied => VerifierFailure::Busy,
        RegistryError::Poisoned => VerifierFailure::Poisoned,
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
