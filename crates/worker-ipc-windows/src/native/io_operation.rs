// SPDX-License-Identifier: MIT
#![allow(unsafe_code)]

//! Stable ownership and completion classification for one Windows operation.

use std::mem::zeroed;
use std::ptr::{addr_of_mut, null, null_mut};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    ERROR_BROKEN_PIPE, ERROR_IO_INCOMPLETE, ERROR_NOT_FOUND, ERROR_OPERATION_ABORTED,
    ERROR_PIPE_NOT_CONNECTED, FALSE, GetLastError, HANDLE, TRUE, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResultEx, OVERLAPPED};
use windows_sys::Win32::System::Threading::CreateEventW;
use zeroize::Zeroizing;

use super::resources::Handle;
use crate::transport::{Deadline, TransportError};

const CLEANUP_GRACE: Duration = Duration::from_secs(1);

/// Stable operation owner.  The heap address of `overlapped`, the event and
/// the zeroizing buffer remain unchanged until `wait` proves a terminal state.
pub(super) struct OperationOwner {
    overlapped: Box<OVERLAPPED>,
    _event: Handle,
    buffer: Zeroizing<Vec<u8>>,
}

impl OperationOwner {
    pub(super) fn new(buffer_len: usize) -> Result<Self, TransportError> {
        let event = unsafe { CreateEventW(null(), TRUE, FALSE, null()) };
        let event = Handle::new(event)?;
        let mut overlapped = Box::new(unsafe { zeroed::<OVERLAPPED>() });
        overlapped.hEvent = event.raw();
        Ok(Self {
            overlapped,
            _event: event,
            buffer: Zeroizing::new(vec![0_u8; buffer_len]),
        })
    }

    pub(super) fn buffer_mut(&mut self) -> &mut [u8] {
        &mut self.buffer
    }

    pub(super) fn buffer(&self) -> &[u8] {
        &self.buffer
    }

    pub(super) fn overlapped_mut(&mut self) -> *mut OVERLAPPED {
        self.overlapped.as_mut()
    }

    /// Waits for completion, transitioning to a bounded cleanup grace after
    /// deadline, cancellation, or an unrecognized result-query error.
    pub(super) fn wait(
        &mut self,
        handle: HANDLE,
        deadline: Deadline,
    ) -> Result<usize, TransportError> {
        self.wait_with(handle, deadline, &NativeCompletion)
    }

    /// The backend is intentionally private to this native module.  Production
    /// code always uses `NativeCompletion`; the generic seam is available only
    /// to sibling unit tests so cancellation/query races can be exercised
    /// without making fault injection runtime-configurable.
    fn wait_with<B: CompletionBackend>(
        &mut self,
        handle: HANDLE,
        deadline: Deadline,
        backend: &B,
    ) -> Result<usize, TransportError> {
        let mut cancel_requested = false;
        let mut uncertain = false;
        let mut cleanup_deadline = None;
        loop {
            let wait_millis = if let Some(grace) = cleanup_deadline {
                remaining_millis(grace)?
            } else {
                match deadline.remaining_millis() {
                    Ok(millis) => millis,
                    Err(TransportError::Deadline) => {
                        cleanup_deadline = Some(grace_deadline());
                        cancel_requested = true;
                        uncertain = false;
                        let _ = request_cancel(backend, handle, &self.overlapped, &mut uncertain);
                        continue;
                    }
                    Err(error) => return Err(error),
                }
            };
            let query = backend.query(handle, &self.overlapped, wait_millis);
            match classify_completion(query.completed, query.error, query.transferred) {
                Completion::Complete(count) => {
                    if cancel_requested {
                        return Err(if uncertain {
                            TransportError::Os
                        } else {
                            TransportError::Deadline
                        });
                    }
                    deadline.check()?;
                    return Ok(count);
                }
                Completion::Terminal(error) => {
                    return if uncertain {
                        Err(TransportError::Os)
                    } else {
                        Err(error)
                    };
                }
                Completion::Pending { timeout } => {
                    if timeout || !cancel_requested {
                        if !cancel_requested {
                            cancel_requested = true;
                            uncertain = !timeout;
                            cleanup_deadline = Some(grace_deadline());
                        }
                        let _ = request_cancel(backend, handle, &self.overlapped, &mut uncertain);
                    }
                }
            }
        }
    }
}

struct CompletionQuery {
    completed: windows_sys::core::BOOL,
    error: Option<u32>,
    transferred: u32,
}

trait CompletionBackend {
    fn query(&self, handle: HANDLE, overlapped: &OVERLAPPED, wait_millis: u32) -> CompletionQuery;

    fn cancel(&self, handle: HANDLE, overlapped: &OVERLAPPED) -> Result<(), u32>;
}

struct NativeCompletion;

impl CompletionBackend for NativeCompletion {
    fn query(&self, handle: HANDLE, overlapped: &OVERLAPPED, wait_millis: u32) -> CompletionQuery {
        let mut transferred = 0_u32;
        let completed = unsafe {
            GetOverlappedResultEx(
                handle,
                overlapped,
                addr_of_mut!(transferred),
                wait_millis,
                FALSE,
            )
        };
        let error = if completed == FALSE {
            // Capture the thread-local error immediately, before any other
            // native call can overwrite it.
            Some(unsafe { GetLastError() })
        } else {
            None
        };
        CompletionQuery {
            completed,
            error,
            transferred,
        }
    }

    fn cancel(&self, handle: HANDLE, overlapped: &OVERLAPPED) -> Result<(), u32> {
        if unsafe { CancelIoEx(handle, overlapped) } != FALSE {
            Ok(())
        } else {
            Err(unsafe { GetLastError() })
        }
    }
}

enum Completion {
    Complete(usize),
    Terminal(TransportError),
    Pending { timeout: bool },
}

/// One classifier is used by connect, read and write.  Only documented
/// terminal operation states prove that the kernel no longer owns the
/// operation owner; unknown query errors remain pending/unproven.
fn classify_completion(
    completed: windows_sys::core::BOOL,
    error: Option<u32>,
    transferred: u32,
) -> Completion {
    if completed != FALSE {
        return Completion::Complete(transferred as usize);
    }
    match error {
        Some(ERROR_OPERATION_ABORTED) => Completion::Terminal(TransportError::Closed),
        Some(ERROR_BROKEN_PIPE | ERROR_PIPE_NOT_CONNECTED) => {
            Completion::Terminal(TransportError::Closed)
        }
        Some(WAIT_TIMEOUT) => Completion::Pending { timeout: true },
        Some(ERROR_IO_INCOMPLETE) => Completion::Pending { timeout: false },
        Some(_) | None => Completion::Pending { timeout: false },
    }
}

/// Cancellation is a request only.  `ERROR_NOT_FOUND` still requires the
/// caller to keep querying; every other error is retained as unproven.
fn request_cancel<B: CompletionBackend>(
    backend: &B,
    handle: HANDLE,
    overlapped: &OVERLAPPED,
    uncertain: &mut bool,
) -> Result<(), ()> {
    match backend.cancel(handle, overlapped) {
        Ok(()) => Ok(()),
        Err(error) => {
            if error != ERROR_NOT_FOUND {
                // INVALID_HANDLE is deliberately not treated as completion.
                // The bounded query grace below either proves a terminal
                // result or invokes fail-stop while this owner is still alive.
                *uncertain = true;
            }
            Err(())
        }
    }
}

pub(super) fn cancel_all(handle: HANDLE) -> Result<(), TransportError> {
    if unsafe { CancelIoEx(handle, null_mut()) } != FALSE {
        Ok(())
    } else {
        let error = unsafe { GetLastError() };
        if error == ERROR_NOT_FOUND {
            Ok(())
        } else {
            Err(TransportError::Os)
        }
    }
}

pub(super) fn cleanup_grace() -> Duration {
    CLEANUP_GRACE
}

fn grace_deadline() -> Instant {
    match Instant::now().checked_add(CLEANUP_GRACE) {
        Some(deadline) => deadline,
        None => fail_stop(),
    }
}

fn remaining_millis(deadline: Instant) -> Result<u32, TransportError> {
    let duration = match deadline.checked_duration_since(Instant::now()) {
        Some(duration) => duration,
        None => fail_stop(),
    };
    let millis = duration.as_millis().saturating_add(1);
    u32::try_from(millis.min(u128::from(u32::MAX))).map_err(|_| TransportError::Os)
}

/// Fatal containment for an operation whose completion cannot be proven.
/// `abort` does not unwind or run operation-owner/destructor code.
pub(super) fn fail_stop<T>() -> T {
    std::process::abort()
}

#[cfg(test)]
#[path = "io_operation_tests.rs"]
mod tests;
