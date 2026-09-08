// SPDX-License-Identifier: MIT
#![allow(unsafe_code)]

use std::cell::Cell;
use std::rc::Rc;

use super::io;
use super::process::{check_worker_sid, verify_peer};
use super::resources::{Handle, HeldImage, ProtectedCredential, ProtectedFile};
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
    active: Rc<Cell<bool>>,
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
            active: Rc::new(Cell::new(false)),
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
        if self.active.get() {
            return Err(TransportError::Busy);
        }
        deadline.check()?;
        let pipe = match self.pipe.take() {
            Some(pipe) => pipe,
            None => io::create_pipe(&self.pipe_name, &self.security, false)?,
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
        self.active.set(true);
        Ok(Connection {
            pipe,
            _process: peer.process,
            _image: peer.image,
            active: Rc::clone(&self.active),
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
        Ok(())
    }

    fn rearm(&mut self) {
        if self.shutdown || self.pipe.is_some() {
            return;
        }
        if let Ok(pipe) = io::create_pipe(&self.pipe_name, &self.security, false) {
            self.pipe = Some(pipe);
        }
    }
}

pub(crate) struct Connection {
    pipe: Handle,
    _process: Handle,
    _image: ProtectedFile,
    active: Rc<Cell<bool>>,
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
        let result = io::read_frame(self.pipe.raw(), limit, self.deadline);
        self.request_read = true;
        if result.is_err() {
            self.close_inner();
        }
        result
    }

    pub(crate) fn write_frame(&mut self, body: &[u8], limit: usize) -> Result<(), TransportError> {
        if self.closed || !self.request_read || self.response_written {
            return Err(TransportError::Closed);
        }
        if body.is_empty() || body.len() > limit || limit == 0 || limit > MAX_FRAME_BYTES {
            self.close_inner();
            return Err(TransportError::Framing);
        }
        let result = io::write_frame(self.pipe.raw(), body, limit, self.deadline);
        self.response_written = true;
        self.close_inner();
        result
    }

    pub(crate) fn close(&mut self) -> Result<(), TransportError> {
        if !self.closed {
            self.close_inner();
        }
        Ok(())
    }

    fn close_inner(&mut self) {
        if self.closed {
            return;
        }
        io::disconnect(self.pipe.raw());
        self.closed = true;
        self.active.set(false);
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.close_inner();
    }
}
