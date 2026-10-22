// SPDX-License-Identifier: MIT

//! Process-local ownership of management supervisors, including abandoned pending shutdowns.

use std::time::Instant;

use super::*;

/// A pending server retains its place even when the caller drops its public handle.
const MAX_SUPERVISED_SERVERS: usize = 128;
static OWNERS: Mutex<Vec<Arc<Supervisor>>> = Mutex::new(Vec::new());

pub(super) struct Supervisor {
    state: Mutex<State>,
    stop: Arc<AtomicBool>,
    pub(super) dispatch: Arc<dispatch_executor::DispatchExecutor>,
}

struct State {
    join: Option<JoinHandle<Result<(), HttpError>>>,
    result: Option<Result<(), HttpError>>,
}

impl Supervisor {
    fn poll(&self) -> Result<Option<Result<(), HttpError>>, HttpError> {
        let mut state = self.state.lock().map_err(|_| unavailable())?;
        if state.join.as_ref().is_some_and(JoinHandle::is_finished) {
            let join = state.join.take().ok_or_else(unavailable)?;
            state.result = Some(join.join().unwrap_or_else(|_| {
                Err(HttpError::new(
                    "server_thread_panicked",
                    "management supervisor terminated unexpectedly",
                ))
            }));
        }
        Ok(state.result.clone())
    }

    pub(super) fn wait(
        &self,
        deadline: Option<Instant>,
    ) -> Result<Option<Result<(), HttpError>>, HttpError> {
        loop {
            if let Some(result) = self.poll()? {
                return Ok(Some(result));
            }
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return Ok(None);
            }
            thread::sleep(Duration::from_millis(1));
        }
    }
}

/// Admission and registration occur under the same lock, before the caller can lose ownership.
pub(super) fn start(
    stop: Arc<AtomicBool>,
    dispatch: Arc<dispatch_executor::DispatchExecutor>,
    task: impl FnOnce() -> Result<(), HttpError> + Send + 'static,
) -> Result<Arc<Supervisor>, HttpError> {
    let mut owners = OWNERS.lock().map_err(|_| unavailable())?;
    start_owned(&mut owners, MAX_SUPERVISED_SERVERS, stop, dispatch, task)
}

fn start_owned(
    owners: &mut Vec<Arc<Supervisor>>,
    capacity: usize,
    stop: Arc<AtomicBool>,
    dispatch: Arc<dispatch_executor::DispatchExecutor>,
    task: impl FnOnce() -> Result<(), HttpError> + Send + 'static,
) -> Result<Arc<Supervisor>, HttpError> {
    reap(owners);
    if owners.len() >= capacity {
        return Err(HttpError::from_management(ManagementError::unavailable(
            "server_supervision_capacity_exhausted",
            "management server supervision is full; retained work must settle before another server starts",
        )));
    }
    let join = thread::Builder::new()
        .name("sts2-management-server".to_owned())
        .spawn(task)
        .map_err(io_http_error)?;
    let owner = Arc::new(Supervisor {
        state: Mutex::new(State {
            join: Some(join),
            result: None,
        }),
        stop,
        dispatch,
    });
    owners.push(Arc::clone(&owner));
    Ok(owner)
}

/// Completed supervisors have already joined their connections and dispatch handlers.
fn reap(owners: &mut Vec<Arc<Supervisor>>) {
    // An unreadable/poisoned owner stays retained and continues occupying capacity.
    owners.retain(|owner| !matches!(owner.poll(), Ok(Some(_))));
}

pub(super) fn reap_retired() -> Result<usize, HttpError> {
    let mut owners = OWNERS.lock().map_err(|_| unavailable())?;
    reap(&mut owners);
    Ok(owners
        .iter()
        .filter(|owner| owner.stop.load(Ordering::Acquire))
        .count())
}

fn unavailable() -> HttpError {
    HttpError::from_management(ManagementError::unavailable(
        "server_supervision_unavailable",
        "management supervision state is unavailable; retained ownership is not released",
    ))
}

#[cfg(test)]
#[path = "http_supervision_tests.rs"]
mod tests;
