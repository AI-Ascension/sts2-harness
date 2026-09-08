// SPDX-License-Identifier: MIT
#![allow(unsafe_code)]

use std::sync::Arc;

use super::connection::{ConnectionResources, ConnectionState};
use super::io;
use super::process::{check_worker_sid, verify_peer};
use super::resources::{Handle, HeldImage, ProtectedCredential};
use super::security::PipeSecurity;
use crate::MAX_FRAME_BYTES;
use crate::policy::EndpointPolicy;
use crate::transport::{Deadline, PeerWitness, TransportError};

pub(crate) struct Listener {
    pipe_name: String,
    expected_sid: String,
    expected_pid: u32,
    expected_creation_filetime: u64,
    security: PipeSecurity,
    credential: ProtectedCredential,
    image: HeldImage,
    pipe: Option<Handle>,
    active: Option<Arc<ConnectionState>>,
    shutdown: bool,
}

impl Listener {
    pub(crate) fn bind(policy: EndpointPolicy) -> Result<Self, TransportError> {
        let expected = policy.expected_peer();
        check_worker_sid(policy.worker_sid().as_str())?;
        let security = PipeSecurity::new(policy.worker_sid().as_str(), expected.sid().as_str())?;
        let credential =
            ProtectedCredential::open(policy.credential_path(), policy.worker_sid().as_str())?;
        let image = HeldImage::open(expected.image_path(), expected.image_sha256())?;
        let pipe = io::create_pipe(policy.pipe_name(), &security, true)?;
        Ok(Self {
            pipe_name: policy.pipe_name().to_owned(),
            expected_sid: expected.sid().as_str().to_owned(),
            expected_pid: expected.pid(),
            expected_creation_filetime: expected.creation_filetime(),
            security,
            credential,
            image,
            pipe: Some(pipe),
            active: None,
            shutdown: false,
        })
    }

    pub(crate) fn accept_authenticated(
        &mut self,
        deadline: Deadline,
    ) -> Result<Connection, TransportError> {
        if self.shutdown {
            return Err(TransportError::Closed);
        }
        if let Some(active) = self.active.as_ref() {
            if !active.can_rearm() {
                return Err(TransportError::Busy);
            }
            // A closed connection retains only an empty shared state.  Drop
            // the listener's reference before creating the next *first*
            // instance; an outstanding connection object may still retain
            // the state without retaining any named-pipe resources.
            self.active.take();
        }
        deadline.check()?;
        let pipe = match self.pipe.take() {
            Some(pipe) => pipe,
            None => io::create_pipe(&self.pipe_name, &self.security, true)?,
        };
        if let Err(error) = io::connect_pipe(pipe.raw(), deadline) {
            io::disconnect(pipe.raw());
            drop(pipe);
            self.rearm();
            return Err(error);
        }
        let peer = match verify_peer(
            pipe.raw(),
            &self.expected_sid,
            self.expected_pid,
            self.expected_creation_filetime,
            &self.image,
            deadline,
        ) {
            Ok(peer) => peer,
            Err(error) => {
                io::disconnect(pipe.raw());
                drop(pipe);
                self.rearm();
                return Err(error);
            }
        };
        if let Err(error) = io::authenticate(pipe.raw(), &self.credential, deadline) {
            io::disconnect(pipe.raw());
            drop(peer);
            drop(pipe);
            self.rearm();
            return Err(error);
        }
        let state = ConnectionState::new(ConnectionResources::new(pipe, peer.process, peer.image));
        self.active = Some(Arc::clone(&state));
        Ok(Connection {
            state,
            witness: PeerWitness::verified(),
            deadline,
            request_read: false,
            response_written: false,
            closed: false,
        })
    }

    pub(crate) fn shutdown(&mut self) -> Result<(), TransportError> {
        if self.shutdown {
            return Ok(());
        }
        self.shutdown = true;
        if let Some(pipe) = self.pipe.take() {
            io::disconnect(pipe.raw());
            drop(pipe);
        }
        self.active.take().map_or(Ok(()), |active| active.close())
    }

    fn rearm(&mut self) {
        if self.shutdown || self.pipe.is_some() {
            return;
        }
        // Every new instance must retain first-instance ownership.  Passing
        // false after the original handle was consumed permits a process that
        // raced the gap to create the endpoint under the same name.
        if let Ok(pipe) = io::create_pipe(&self.pipe_name, &self.security, true) {
            self.pipe = Some(pipe);
        }
    }
}

pub(crate) struct Connection {
    state: Arc<ConnectionState>,
    witness: PeerWitness,
    deadline: Deadline,
    request_read: bool,
    response_written: bool,
    closed: bool,
}

impl Connection {
    pub(crate) fn peer_witness(&self) -> &PeerWitness {
        &self.witness
    }

    pub(crate) fn read_frame(&mut self, limit: usize) -> Result<Vec<u8>, TransportError> {
        if self.closed || self.request_read || limit == 0 || limit > MAX_FRAME_BYTES {
            return Err(TransportError::Closed);
        }
        let operation = self.state.begin()?;
        let result = io::read_frame(operation.handle(), limit, self.deadline);
        // The overlapped buffer is owned by read_frame and cannot be released
        // until read_frame has returned.  Dropping this guard then publishes
        // completion to listener shutdown.  The atomic finish also makes a
        // concurrent close generation win over an otherwise completed read.
        let current = operation.finish();
        let result = if current {
            result
        } else {
            Err(TransportError::Closed)
        };
        self.request_read = true;
        if result.is_err() {
            let _ = self.close_inner();
        }
        result
    }

    pub(crate) fn write_frame(&mut self, body: &[u8], limit: usize) -> Result<(), TransportError> {
        if self.closed || !self.request_read || self.response_written {
            return Err(TransportError::Closed);
        }
        if body.is_empty() || body.len() > limit || limit == 0 || limit > MAX_FRAME_BYTES {
            let _ = self.close_inner();
            return Err(TransportError::Framing);
        }
        let operation = self.state.begin()?;
        let result = io::write_frame(operation.handle(), body, limit, self.deadline);
        let current = operation.finish();
        let result = if current {
            result
        } else {
            Err(TransportError::Closed)
        };
        self.response_written = true;
        let close_result = self.close_inner();
        match (result, close_result) {
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
            (Ok(()), Ok(())) => Ok(()),
        }
    }

    pub(crate) fn close(&mut self) -> Result<(), TransportError> {
        self.close_inner()
    }

    fn close_inner(&mut self) -> Result<(), TransportError> {
        if self.closed {
            return self.state.close();
        }
        self.closed = true;
        self.state.close()
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.close_inner();
    }
}
