// SPDX-License-Identifier: MIT

//! A finite owner for synchronous dispatch work. A deadline ends the HTTP wait,
//! never cancels an accepted write or releases the underlying worker's slot.

use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Instant;

use super::*;
use route_effects::RouteEffect;

type DispatchOutcome = std::thread::Result<Result<HttpResponse, HttpError>>;

pub(super) struct DispatchExecutor {
    workers: Mutex<Vec<(bool, JoinHandle<()>)>>,
    capacity: usize,
}

impl DispatchExecutor {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            workers: Mutex::new(Vec::new()),
            capacity,
        }
    }

    pub(super) fn dispatch(
        &self,
        request: HttpRequest,
        service: Arc<ManagementService>,
        authenticator: Arc<dyn Authenticator>,
        budget: Duration,
    ) -> DispatchOutcome {
        let deadline = Instant::now() + budget;
        let effect = route_effects::classify(&request);
        let recovery = route_effects::is_submission_recovery(&request);
        let (sender, receiver) = mpsc::sync_channel(1);
        let mut workers = match self.workers.lock() {
            Ok(workers) => workers,
            Err(_) => return Ok(Err(unavailable("dispatch_owner_poisoned"))),
        };
        reap(&mut workers);
        let ordinary = workers.iter().filter(|(recovery, _)| !recovery).count();
        if workers.len() >= self.capacity
            || (!recovery && ordinary >= self.capacity.saturating_sub(1).max(1))
        {
            return Ok(Err(unavailable("dispatch_capacity_exhausted")));
        }
        let worker = thread::Builder::new()
            .name("sts2-management-dispatch".to_owned())
            .spawn(move || {
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    routes::dispatch(request, &service, &*authenticator)
                }));
                // A timed-out peer is gone. Work still finishes its durable claim. A
                // late panic remains a thread failure rather than being silently swallowed.
                if let Err(mpsc::SendError(Err(payload))) = sender.send(outcome) {
                    std::panic::resume_unwind(payload);
                }
            });
        match worker {
            Ok(worker) => workers.push((recovery, worker)),
            Err(_) => return Ok(Err(unavailable("dispatch_worker_unavailable"))),
        }
        drop(workers);
        await_outcome(receiver, deadline, effect)
    }

    /// The retained server supervisor owns this drain. Public shutdown waits independently and
    /// can return pending; it never discards this worker vector or releases accepted capacity.
    pub(super) fn drain(&self) {
        loop {
            // Poison refuses new admission, but does not invalidate Rust's owned JoinHandles.
            // Teardown still owns and joins every retained handle instead of detaching them.
            let mut workers = self
                .workers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            reap(&mut workers);
            if workers.is_empty() {
                return;
            }
            drop(workers);
            thread::sleep(Duration::from_millis(1));
        }
    }

    pub(super) fn pending(&self) -> Result<usize, HttpError> {
        let mut workers = self
            .workers
            .lock()
            .map_err(|_| unavailable("dispatch_owner_poisoned"))?;
        reap(&mut workers);
        Ok(workers.len())
    }
}

fn await_outcome(
    receiver: mpsc::Receiver<DispatchOutcome>,
    deadline: Instant,
    effect: RouteEffect,
) -> DispatchOutcome {
    match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
        Ok(outcome) => outcome,
        Err(RecvTimeoutError::Timeout) => Ok(Err(wait_failed(effect, false))),
        Err(RecvTimeoutError::Disconnected) => Ok(Err(wait_failed(effect, true))),
    }
}

fn reap(workers: &mut Vec<(bool, JoinHandle<()>)>) {
    let mut index = 0;
    while index < workers.len() {
        if workers[index].1.is_finished() {
            let _ = workers.swap_remove(index).1.join();
        } else {
            index += 1;
        }
    }
}

fn unavailable(code: &'static str) -> HttpError {
    HttpError::from_management(ManagementError::unavailable(
        code,
        "management dispatch is unavailable; no additional handler was started",
    ))
}

fn wait_failed(effect: RouteEffect, disconnected: bool) -> HttpError {
    let code = if disconnected {
        "dispatch_worker_lost"
    } else {
        "dispatch_deadline_exceeded"
    };
    let error = match effect {
        RouteEffect::MayWrite => ManagementError::write_state_unknown(
            code,
            "dispatch outcome is unknown; recover the original request or operation identity before resubmission",
        ),
        RouteEffect::ReadOnly => {
            ManagementError::unavailable(code, "read-only dispatch did not return an answer")
        }
    };
    HttpError::from_management(error)
}

#[cfg(test)]
#[path = "http_dispatch_outcome_tests.rs"]
mod tests;
