// SPDX-License-Identifier: MIT

use std::time::Instant;

use super::*;

pub struct ServerHandle {
    pub(super) address: SocketAddr,
    pub(super) stop: Arc<AtomicBool>,
    pub(super) supervisor: Arc<supervision::Supervisor>,
}

impl ServerHandle {
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// Stops admission and waits up to 100 ms for all accepted work to be joined.
    ///
    /// A pending result keeps this handle usable for inspection, another shutdown call, or wait.
    /// Dropping the handle transfers no work: the bounded process registry already owns the same
    /// supervisor until every connection and dispatch handler has finished and been joined.
    pub fn shutdown(&self) -> Result<(), HttpError> {
        self.request_stop();
        match self
            .supervisor
            .wait(Some(Instant::now() + Duration::from_millis(100)))?
        {
            Some(result) => {
                supervision::reap_retired()?;
                result
            }
            None => Err(HttpError::from_management(
                ManagementError::write_state_unknown(
                    "dispatch_shutdown_pending",
                    "management shutdown retains supervised work; writes are not cancelled; retain this handle and reconcile original identities",
                ),
            )),
        }
    }

    /// Waits for the supervisor to join every accepted handler. An uncooperative port can keep
    /// this explicit wait pending indefinitely; callers needing a bound use [`Self::shutdown`].
    pub fn wait(self) -> Result<(), HttpError> {
        let result = self.supervisor.wait(None)?.ok_or_else(|| {
            HttpError::new(
                "server_join_missing",
                "management supervisor result is missing",
            )
        })?;
        supervision::reap_retired()?;
        result
    }

    /// Reaps completed dispatch workers and returns the number whose capacity remains held.
    pub fn pending_dispatches(&self) -> Result<usize, HttpError> {
        self.supervisor.dispatch.pending()
    }

    fn request_stop(&self) {
        if !self.stop.swap(true, Ordering::AcqRel) {
            wake_listener(self.address);
        }
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.request_stop();
        // The registry keeps the supervisor and all unfinished worker handles. Its finite
        // admission bound is released only after a later inspection/start reaps completed work.
    }
}
