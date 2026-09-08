// SPDX-License-Identifier: MIT

use super::*;

impl WorkerRuntime {
    /// Completes the response-before-start boundary.  A failed response drops the permit and
    /// durably retains the admitted handoff as unknown; it is never eligible for a fresh retry.
    pub fn finish_exchange(
        &mut self,
        exchange: WorkerExchange,
        response: ResponseWriteStatus,
    ) -> Result<WorkerStartOutcome, String> {
        let (_, reservation) = exchange.into_parts();
        self.finish_reservation(reservation, response)
    }

    /// Finishes an exchange after the caller has moved its reply into the wire encoder. Keeping
    /// the reservation separate lets the endpoint retain it when response encoding itself fails.
    pub fn finish_reservation(
        &mut self,
        reservation: Option<WorkerExecutionReservation>,
        response: ResponseWriteStatus,
    ) -> Result<WorkerStartOutcome, String> {
        let Some(reservation) = reservation else {
            return Ok(WorkerStartOutcome::NoExecution);
        };
        let handoff_id = reservation.tuple().handoff_id.clone();
        if response == ResponseWriteStatus::Failed {
            drop(reservation);
            return match self.retain_unknown(&handoff_id) {
                Ok(()) => Ok(WorkerStartOutcome::Unknown { handoff_id }),
                Err(error) => Err(format!(
                    "worker response write failed; failed to retain unknown handoff: {error}"
                )),
            };
        }
        let mut store = match try_lock(&self.store) {
            Ok(store) => store,
            Err(error) => {
                // The response already crossed the admission boundary. The permit cannot be
                // retried or silently dropped when the normal lease is unavailable: preserve
                // the reservation as unknown through the recovery lease, while retaining the
                // original lock/gate diagnostic if that write is also blocked.
                drop(reservation);
                return match self.retain_unknown(&handoff_id) {
                    Ok(()) => Ok(WorkerStartOutcome::Unknown { handoff_id }),
                    Err(quarantine) => Err(combine_failure(error, Err(quarantine))),
                };
            }
        };
        match reservation.start(&mut store) {
            Ok(running) => {
                if running.state != WorkerHandoffState::Running {
                    drop(store);
                    return Err(combine_failure(
                        String::from("worker reservation did not reach the running state"),
                        self.retain_unknown(&handoff_id),
                    ));
                }
                Ok(WorkerStartOutcome::Started(Box::new(running)))
            }
            Err(error) => {
                drop(store);
                Err(combine_failure(
                    format!("worker reservation could not start: {error}"),
                    self.retain_unknown(&handoff_id),
                ))
            }
        }
    }

    pub fn retain_unknown(&self, handoff_id: &str) -> Result<(), String> {
        retain_unknown(&self.store, handoff_id)
    }

    /// Releases the capacity-one lane only after the same durable handoff has a projected
    /// terminal receipt. Unknown, admitted, and running rows remain lookup-only.
    pub fn release_completed(&mut self, handoff_id: &str) -> Result<(), String> {
        if self.lane.execution_taken {
            return Err(String::from(
                "worker execution completion is still owned by its task",
            ));
        }
        self.release_durable_completion(handoff_id)
    }

    pub(super) fn release_durable_completion(&mut self, handoff_id: &str) -> Result<(), String> {
        let tuple = self
            .lane
            .tuple()
            .filter(|tuple| tuple.handoff_id == handoff_id)
            .cloned()
            .ok_or_else(|| String::from("worker completion does not match active lane"))?;
        let mut store = try_lock(&self.store)?;
        let handoff = match store
            .lookup_worker_handoff(&tuple)
            .map_err(|_| String::from("cannot reconcile worker completion"))?
        {
            crate::WorkerLookup::Known(handoff) => *handoff,
            crate::WorkerLookup::Unknown { .. } => {
                return Err(String::from("worker completion handoff is not retained"));
            }
        };
        if handoff.terminal.is_none() {
            return Err(String::from(
                "worker execution cannot release its lane before durable completion",
            ));
        }
        drop(store);
        self.lane.release(handoff_id)
    }

    pub fn close(&self) -> Result<(), String> {
        if self.lane.execution_taken {
            return Err(String::from(
                "cannot close worker store while execution is owned",
            ));
        }
        let mut store = try_lock_close(&self.store)?;
        store
            .close()
            .map_err(|_| String::from("cannot close worker execution store"))
    }
}

pub(super) fn retain_unknown(store: &SharedExecutionStore, handoff_id: &str) -> Result<(), String> {
    // Latch the admission fence before attempting the recovery write. Once a runtime has
    // crossed an uncertainty boundary, no concurrent dispatch may reopen the lane while this
    // handoff is being retained. The recovery lease deliberately bypasses that fence so an
    // already-running quarantine can still account for the existing handoff.
    let _already_quarantined = begin_quarantine(store)?;
    let mut lease = try_lock_quarantine(store).map_err(|error| {
        format!("cannot acquire worker recovery lease for unknown handoff: {error}")
    })?;
    let result = lease
        .mark_worker_handoff_unknown(handoff_id)
        .map(|_| ())
        .map_err(|error| format!("cannot retain uncertain worker handoff: {error}"));
    if result.is_ok() {
        finish_quarantine(store);
    }
    result
}
