// SPDX-License-Identifier: MIT

//! A finite owner for synchronous dispatch work. A deadline ends the HTTP wait,
//! never cancels an accepted write or releases the underlying worker's slot.

use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Instant;

use super::*;

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
    ) -> std::thread::Result<Result<HttpResponse, HttpError>> {
        let deadline = Instant::now() + budget;
        let mutation = request.method != "GET";
        let recovery =
            request.method == "GET" && request.path.starts_with("/v1/workflow-submissions/");
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
        match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(outcome) => outcome,
            Err(RecvTimeoutError::Timeout) => Ok(Err(timeout(mutation))),
            Err(RecvTimeoutError::Disconnected) => Ok(Err(HttpError::from_management(
                ManagementError::write_state_unknown(
                    "dispatch_worker_lost",
                    "dispatch outcome is unknown; recover the original operation identity",
                ),
            ))),
        }
    }

    /// Never join an uncooperative synchronous port indefinitely. Finished work
    /// is joined; remaining work is reported explicitly and keeps its claims.
    pub(super) fn shutdown(&self) -> Result<(), HttpError> {
        let deadline = Instant::now() + Duration::from_millis(100);
        loop {
            if self.reap_shutdown()? {
                return Ok(());
            }
            if Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        Err(HttpError::from_management(
            ManagementError::write_state_unknown(
                "dispatch_shutdown_pending",
                "synchronous dispatch work remains; its writes are not cancelled and require reconciliation",
            ),
        ))
    }

    fn reap_shutdown(&self) -> Result<bool, HttpError> {
        let mut workers = self
            .workers
            .lock()
            .map_err(|_| unavailable("dispatch_owner_poisoned"))?;
        reap(&mut workers);
        Ok(workers.is_empty())
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
        "management dispatch is unavailable; no handler was started",
    ))
}

fn timeout(mutation: bool) -> HttpError {
    let error = if mutation {
        ManagementError::write_state_unknown(
            "dispatch_deadline_exceeded",
            "dispatch wait expired; recover the original request or operation identity before resubmission",
        )
    } else {
        ManagementError::unavailable(
            "dispatch_deadline_exceeded",
            "read-only dispatch wait expired",
        )
    };
    HttpError::from_management(error)
}
