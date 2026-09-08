// SPDX-License-Identifier: MIT
#![allow(unsafe_code)]

//! Deterministic completion/cancellation fault tests for the native seam.

use std::collections::VecDeque;
use std::ptr::null_mut;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    ERROR_IO_INCOMPLETE, ERROR_NOT_FOUND, ERROR_OPERATION_ABORTED, FALSE, HANDLE, TRUE,
    WAIT_TIMEOUT,
};

use super::{Completion, CompletionBackend, CompletionQuery, OperationOwner, classify_completion};
use crate::transport::{Deadline, TransportError};

struct FakeCompletion {
    queries: Mutex<VecDeque<CompletionQuery>>,
    cancellations: Mutex<VecDeque<Result<(), u32>>>,
    query_count: AtomicUsize,
    cancel_count: AtomicUsize,
}

impl FakeCompletion {
    fn new(
        queries: impl IntoIterator<Item = CompletionQuery>,
        cancellations: impl IntoIterator<Item = Result<(), u32>>,
    ) -> Self {
        Self {
            queries: Mutex::new(queries.into_iter().collect()),
            cancellations: Mutex::new(cancellations.into_iter().collect()),
            query_count: AtomicUsize::new(0),
            cancel_count: AtomicUsize::new(0),
        }
    }

    fn query_count(&self) -> usize {
        self.query_count.load(Ordering::Relaxed)
    }

    fn cancel_count(&self) -> usize {
        self.cancel_count.load(Ordering::Relaxed)
    }
}

impl CompletionBackend for FakeCompletion {
    fn query(
        &self,
        _handle: HANDLE,
        _overlapped: &windows_sys::Win32::System::IO::OVERLAPPED,
        _wait_millis: u32,
    ) -> CompletionQuery {
        self.query_count.fetch_add(1, Ordering::Relaxed);
        match self.queries.lock() {
            Ok(mut queries) => match queries.pop_front() {
                Some(query) => query,
                None => CompletionQuery {
                    completed: FALSE,
                    error: Some(ERROR_IO_INCOMPLETE),
                    transferred: 0,
                },
            },
            Err(_) => CompletionQuery {
                completed: FALSE,
                error: Some(ERROR_IO_INCOMPLETE),
                transferred: 0,
            },
        }
    }

    fn cancel(
        &self,
        _handle: HANDLE,
        _overlapped: &windows_sys::Win32::System::IO::OVERLAPPED,
    ) -> Result<(), u32> {
        self.cancel_count.fetch_add(1, Ordering::Relaxed);
        match self.cancellations.lock() {
            Ok(mut cancellations) => match cancellations.pop_front() {
                Some(result) => result,
                None => Ok(()),
            },
            Err(_) => Err(ERROR_OPERATION_ABORTED),
        }
    }
}

fn query(error: Option<u32>, transferred: u32) -> CompletionQuery {
    CompletionQuery {
        completed: FALSE,
        error,
        transferred,
    }
}

#[test]
fn classifier_keeps_unknown_query_errors_pending() {
    assert!(matches!(
        classify_completion(FALSE, Some(0xdead), 0),
        Completion::Pending { timeout: false }
    ));
    assert!(matches!(
        classify_completion(FALSE, Some(ERROR_IO_INCOMPLETE), 0),
        Completion::Pending { timeout: false }
    ));
}

#[test]
fn cancellation_not_found_still_queries_for_completion() -> Result<(), TransportError> {
    let fake = FakeCompletion::new(
        [
            query(Some(WAIT_TIMEOUT), 0),
            CompletionQuery {
                completed: TRUE,
                error: None,
                transferred: 7,
            },
        ],
        [Err(ERROR_NOT_FOUND)],
    );
    let mut owner = OperationOwner::new(0)?;
    let deadline = Deadline::new(Duration::from_secs(1))?;
    let result = owner.wait_with(null_mut(), deadline, &fake);
    assert!(matches!(result, Err(TransportError::Deadline)));
    assert_eq!(fake.query_count(), 2);
    assert_eq!(fake.cancel_count(), 1);
    Ok(())
}

#[test]
fn deadline_cancellation_retains_deadline_after_abort_completion() -> Result<(), TransportError> {
    let fake = FakeCompletion::new(
        [
            query(Some(WAIT_TIMEOUT), 0),
            query(Some(ERROR_OPERATION_ABORTED), 0),
        ],
        [Ok(())],
    );
    let mut owner = OperationOwner::new(0)?;
    let result = owner.wait_with(null_mut(), Deadline::new(Duration::from_secs(1))?, &fake);
    assert!(matches!(result, Err(TransportError::Deadline)));
    assert_eq!(fake.query_count(), 2);
    assert_eq!(fake.cancel_count(), 1);
    Ok(())
}

#[test]
fn unknown_cancel_error_remains_unproven_until_terminal_query() -> Result<(), TransportError> {
    let fake = FakeCompletion::new(
        [
            query(Some(0xdead), 0),
            query(Some(ERROR_OPERATION_ABORTED), 0),
        ],
        [Err(0xbeef)],
    );
    let mut owner = OperationOwner::new(0)?;
    let deadline = Deadline::new(Duration::from_secs(1))?;
    let result = owner.wait_with(null_mut(), deadline, &fake);
    assert!(matches!(result, Err(TransportError::Os)));
    assert_eq!(fake.query_count(), 2);
    assert_eq!(fake.cancel_count(), 1);
    Ok(())
}
