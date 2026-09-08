// SPDX-License-Identifier: MIT
#![allow(unsafe_code)]

//! Shared ownership and cancellation state for one authenticated exchange.
//!
//! A listener and its returned connection both retain the same state.  The
//! listener can therefore request cancellation while a connection is blocked
//! in overlapped I/O, then wait for the operation guard before releasing the
//! pipe, process handle, image handle, or any heap-owned overlapped buffer.

use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use windows_sys::Win32::Foundation::HANDLE;

use super::io;
use super::io_operation;
use super::resources::{Handle, ProtectedFile};
use crate::transport::TransportError;

pub(super) struct ConnectionResources {
    pipe: Option<Handle>,
    process: Option<Handle>,
    image: Option<ProtectedFile>,
}

impl ConnectionResources {
    pub(super) fn new(pipe: Handle, process: Handle, image: ProtectedFile) -> Self {
        Self {
            pipe: Some(pipe),
            process: Some(process),
            image: Some(image),
        }
    }

    fn pipe_raw(&self) -> Result<HANDLE, TransportError> {
        self.pipe
            .as_ref()
            .map(Handle::raw)
            .ok_or(TransportError::Closed)
    }

    fn is_empty(&self) -> bool {
        self.pipe.is_none() && self.process.is_none() && self.image.is_none()
    }

    fn release(&mut self) {
        // Drop the pipe last.  The operation join has already completed when
        // this is called, so no native call can retain a pointer into an I/O
        // buffer or use the handle after this point.
        self.process.take();
        self.image.take();
        self.pipe.take();
    }
}

#[derive(Default)]
struct ConnectionStatus {
    closed: bool,
    cleanup_started: bool,
    cleanup_done: bool,
    in_flight: usize,
    generation: u64,
    close_error: Option<TransportError>,
}

/// State shared by the listener and one returned connection.
pub(super) struct ConnectionState {
    status: Mutex<ConnectionStatus>,
    resources: Mutex<ConnectionResources>,
    changed: Condvar,
}

impl ConnectionState {
    pub(super) fn new(resources: ConnectionResources) -> Arc<Self> {
        Arc::new(Self {
            status: Mutex::new(ConnectionStatus::default()),
            resources: Mutex::new(resources),
            changed: Condvar::new(),
        })
    }

    /// Begins one native operation and snapshots the still-owned pipe handle.
    /// The status lock is held while taking the resource lock so shutdown
    /// cannot mark the state closed between the liveness check and snapshot.
    pub(super) fn begin(self: &Arc<Self>) -> Result<Operation, TransportError> {
        let mut status = self.status.lock().map_err(|_| TransportError::Closed)?;
        if status.closed {
            return Err(TransportError::Closed);
        }
        let generation = status.generation;
        let resources = self.resources.lock().map_err(|_| TransportError::Closed)?;
        let handle = resources.pipe_raw()?;
        status.in_flight = status
            .in_flight
            .checked_add(1)
            .ok_or(TransportError::Closed)?;
        Ok(Operation {
            state: Arc::clone(self),
            handle,
            generation,
            finished: false,
        })
    }

    /// Requests cancellation, disconnects the named pipe, joins every
    /// operation guard, and only then drops all owned native resources.
    pub(super) fn close(&self) -> Result<(), TransportError> {
        let owner = {
            let mut status = match self.status.lock() {
                Ok(status) => status,
                Err(_) => io_operation::fail_stop(),
            };
            if status.cleanup_done {
                return status.close_error.map_or(Ok(()), Err);
            }
            status.closed = true;
            status.generation = status.generation.wrapping_add(1);
            if status.cleanup_started {
                false
            } else {
                status.cleanup_started = true;
                true
            }
        };

        if !owner {
            return self.wait_for_cleanup();
        }

        let handle = match self.resources.lock() {
            Ok(resources) => resources.pipe_raw().ok(),
            Err(_) => io_operation::fail_stop(),
        };

        // Cancellation is a request only.  Regardless of whether the request
        // reports an OS error, wait_for_operations below is mandatory before
        // releasing the OVERLAPPED-owned pipe and peer resources.
        let mut first_error = handle.and_then(|raw| io::cancel_all(raw).err());
        if let Some(raw) = handle {
            io::disconnect(raw);
        }
        if let Err(error) = self.wait_for_operations()
            && first_error.is_none()
        {
            first_error = Some(error);
        }
        let mut resources = match self.resources.lock() {
            Ok(resources) => resources,
            Err(_) => io_operation::fail_stop(),
        };
        resources.release();

        let result = first_error.map_or(Ok(()), Err);
        let mut status = match self.status.lock() {
            Ok(status) => status,
            Err(_) => io_operation::fail_stop(),
        };
        status.close_error = result.err();
        status.cleanup_done = true;
        self.changed.notify_all();
        status.close_error.map_or(Ok(()), Err)
    }

    pub(super) fn can_rearm(&self) -> bool {
        let status = match self.status.lock() {
            Ok(status) => status,
            Err(_) => return false,
        };
        if !status.cleanup_done || status.in_flight != 0 {
            return false;
        }
        let resources = match self.resources.lock() {
            Ok(resources) => resources,
            Err(_) => return false,
        };
        resources.is_empty()
    }

    fn wait_for_operations(&self) -> Result<(), TransportError> {
        let grace_deadline = match Instant::now().checked_add(io_operation::cleanup_grace()) {
            Some(deadline) => deadline,
            None => io_operation::fail_stop(),
        };
        let mut status = match self.status.lock() {
            Ok(status) => status,
            Err(_) => io_operation::fail_stop(),
        };
        while status.in_flight != 0 {
            let remaining = match grace_deadline.checked_duration_since(Instant::now()) {
                Some(remaining) if !remaining.is_zero() => remaining,
                _ => io_operation::fail_stop(),
            };
            let (next, timeout) = match self.changed.wait_timeout(status, remaining) {
                Ok(result) => result,
                Err(_) => io_operation::fail_stop(),
            };
            status = next;
            if timeout.timed_out() && status.in_flight != 0 {
                return io_operation::fail_stop();
            }
        }
        Ok(())
    }

    fn wait_for_cleanup(&self) -> Result<(), TransportError> {
        let grace_deadline = match Instant::now().checked_add(io_operation::cleanup_grace()) {
            Some(deadline) => deadline,
            None => io_operation::fail_stop(),
        };
        let mut status = match self.status.lock() {
            Ok(status) => status,
            Err(_) => io_operation::fail_stop(),
        };
        while !status.cleanup_done {
            let remaining = match grace_deadline.checked_duration_since(Instant::now()) {
                Some(remaining) if !remaining.is_zero() => remaining,
                _ => io_operation::fail_stop(),
            };
            let (next, timeout) = match self.changed.wait_timeout(status, remaining) {
                Ok(result) => result,
                Err(_) => io_operation::fail_stop(),
            };
            status = next;
            if timeout.timed_out() && !status.cleanup_done {
                return io_operation::fail_stop();
            }
        }
        status.close_error.map_or(Ok(()), Err)
    }

    fn finish_operation(&self, generation: u64) -> bool {
        let mut status = match self.status.lock() {
            Ok(status) => status,
            Err(_) => io_operation::fail_stop(),
        };
        let current = !status.closed && status.generation == generation;
        status.in_flight = status.in_flight.saturating_sub(1);
        if status.in_flight == 0 {
            self.changed.notify_all();
        }
        current
    }
}

/// Operation guard that keeps the shared resources alive until native I/O has
/// returned.  Its handle is only a borrowed raw value; the owning `Handle`
/// remains in `ConnectionResources` until the guard is dropped.
pub(super) struct Operation {
    state: Arc<ConnectionState>,
    handle: HANDLE,
    generation: u64,
    finished: bool,
}

impl Operation {
    pub(super) fn handle(&self) -> HANDLE {
        self.handle
    }

    /// Atomically publishes native completion and checks that close has not
    /// won the generation race.  The status lock prevents close from marking
    /// this operation stale between the check and the in-flight decrement.
    pub(super) fn finish(mut self) -> bool {
        let current = self.state.finish_operation(self.generation);
        self.finished = true;
        current
    }
}

impl Drop for Operation {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.state.finish_operation(self.generation);
        }
    }
}
