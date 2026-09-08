// SPDX-License-Identifier: MIT

//! Runtime-side worker admission and execution handoff.
//!
//! This module is deliberately split from the native listener.  A listener authenticates one
//! connection and supplies an [`AuthenticatedWorkerRequest`]; this module owns the single shared
//! durable store, response-before-start ordering, and the capacity-one execution lane.  Until a
//! protected native listener is integrated, production worker mode stops after boot persistence
//! with a fixed error rather than fabricating an authenticated peer.

use sts2_harness::worker_handoff::{
    AuthenticatedWorkerRequest, DispatchReply, WorkerCommandAdmission, WorkerCommandConfig,
    WorkerCommandError, WorkerExecutionReservation, WorkerReply,
};
use sts2_harness::{
    ExecutionFingerprint, ExecutionStore, ExecutionStoreConfig, StoredWorkerHandoff,
    WorkerHandoffState, WorkerTuple,
};

use super::super::config::RuntimeConfig;
use super::super::worker_settings::WorkerSettings;
use super::worker_store::{
    SharedExecutionStore, begin_quarantine, finish_quarantine, share_store, try_lock,
    try_lock_quarantine,
};

/// Production does not silently fall back to a fake or unauthenticated worker transport.
pub(super) const MISSING_TRANSPORT_ERROR: &str =
    "worker transport unavailable: authenticated native worker listener is not integrated";

/// Result of a response write as observed by the endpoint.  A failed response after durable
/// admission is an uncertainty boundary and must retain the handoff as unknown.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResponseWriteStatus {
    Written,
    Failed,
}

/// Outcome of taking a fresh reservation past the response boundary.
pub enum WorkerStartOutcome {
    NoExecution,
    Started(Box<StoredWorkerHandoff>),
    Unknown { handoff_id: String },
}

impl WorkerStartOutcome {
    pub fn handoff_id(&self) -> Option<&str> {
        match self {
            Self::NoExecution => None,
            Self::Started(handoff) => Some(handoff.tuple.handoff_id.as_str()),
            Self::Unknown { handoff_id } => Some(handoff_id.as_str()),
        }
    }
}

/// A typed command result plus the one-time reservation, if this exchange won dispatch admission.
/// The reservation is intentionally not started while the connection is being handled.
pub struct WorkerExchange {
    reply: WorkerReply,
    reservation: Option<WorkerExecutionReservation>,
}

impl WorkerExchange {
    pub fn into_parts(self) -> (WorkerReply, Option<WorkerExecutionReservation>) {
        (self.reply, self.reservation)
    }

    fn reply(reply: WorkerReply) -> Self {
        Self {
            reply,
            reservation: None,
        }
    }
}

#[derive(Default)]
struct ExecutionLane {
    active: Option<WorkerTuple>,
}

impl ExecutionLane {
    fn active_handoff_id(&self) -> Option<&str> {
        self.active.as_ref().map(|tuple| tuple.handoff_id.as_str())
    }

    fn occupy(&mut self, tuple: WorkerTuple) -> Result<(), String> {
        if self.active.is_some() {
            return Err(String::from("worker execution lane is busy"));
        }
        self.active = Some(tuple);
        Ok(())
    }

    fn tuple(&self) -> Option<&WorkerTuple> {
        self.active.as_ref()
    }

    fn release(&mut self, handoff_id: &str) -> Result<(), String> {
        if self.active_handoff_id() != Some(handoff_id) {
            return Err(String::from(
                "worker completion does not match the active execution lane",
            ));
        }
        self.active = None;
        Ok(())
    }
}

/// One worker-owned store and one capacity-one runtime lane.
pub struct WorkerRuntime {
    store: SharedExecutionStore,
    admission: WorkerCommandAdmission,
    fingerprint: ExecutionFingerprint,
    worker_boot_id: String,
    lane: ExecutionLane,
}

impl WorkerRuntime {
    /// Opens the one store and persists a stopped, non-admitting worker boot before any listener
    /// could accept a dispatch.  No episode, job, or handoff is claimed by this constructor.
    pub fn open(settings: WorkerSettings) -> Result<Self, String> {
        let store_config =
            ExecutionStoreConfig::new(settings.store_path).with_approved_recovery_schema();
        let mut store = ExecutionStore::open(store_config)
            .map_err(|_| String::from("cannot open worker execution store"))?;
        if let Err(error) = store.start_worker_boot(&settings.boot) {
            let _ = store.close();
            return Err(format!("cannot persist worker boot: {error}"));
        }
        Self::from_shared_store(
            share_store(store),
            settings.command,
            settings.fingerprint,
            settings.boot.worker_boot_id,
        )
    }

    /// Test and native-adapter constructor for an already-open worker-owned store.
    pub(super) fn from_shared_store(
        store: SharedExecutionStore,
        command: WorkerCommandConfig,
        fingerprint: ExecutionFingerprint,
        worker_boot_id: String,
    ) -> Result<Self, String> {
        let admission = WorkerCommandAdmission::new(command)
            .map_err(|error| format!("worker command configuration is invalid: {error}"))?;
        Ok(Self {
            store,
            admission,
            fingerprint,
            worker_boot_id,
            lane: ExecutionLane::default(),
        })
    }

    /// Handles an already-authenticated command.  The native endpoint must retain the original
    /// request for response encoding and call [`Self::finish_exchange`] only after its response
    /// write has completed.
    pub fn handle_authenticated(
        &mut self,
        authenticated: &AuthenticatedWorkerRequest,
    ) -> Result<WorkerExchange, String> {
        if authenticated.request().command()
            == sts2_harness::worker_handoff::WorkerCommand::Dispatch
        {
            return self.handle_dispatch(authenticated);
        }
        let mut store = try_lock(&self.store)?;
        let mut result = self
            .admission
            .handle_authenticated(&mut store, authenticated)
            .map_err(command_error)?;
        let reservation = result.take_reservation();
        let (reply, leftover) = result.into_parts();
        debug_assert!(leftover.is_none());
        if reservation.is_some() {
            return Err(String::from(
                "non-dispatch worker command returned an execution reservation",
            ));
        }
        Ok(WorkerExchange::reply(reply))
    }

    fn handle_dispatch(
        &mut self,
        authenticated: &AuthenticatedWorkerRequest,
    ) -> Result<WorkerExchange, String> {
        let request = authenticated.request();
        if let Some(active_handoff_id) = self.lane.active_handoff_id()
            && request
                .fields()
                .get("handoff_id")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|handoff_id| handoff_id != active_handoff_id)
        {
            return Ok(WorkerExchange::reply(WorkerReply::Dispatch(
                DispatchReply::Busy,
            )));
        }
        let mut store = try_lock(&self.store)?;
        let preparation = self
            .admission
            .prepare_dispatch_from_authenticated(
                &mut store,
                authenticated,
                self.fingerprint.clone(),
            )
            .map_err(command_error)?;
        let mut result = self
            .admission
            .admit_prepared_dispatch(&mut store, authenticated, preparation)
            .map_err(command_error)?;
        let reservation = result.take_reservation();
        let (reply, leftover) = result.into_parts();
        debug_assert!(leftover.is_none());
        if let Some(reservation) = &reservation {
            self.lane.occupy(reservation.tuple().clone())?;
        }
        Ok(WorkerExchange { reply, reservation })
    }

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
        let mut store = try_lock(&self.store)?;
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

    fn retain_unknown(&self, handoff_id: &str) -> Result<(), String> {
        // Latch the admission fence before attempting the recovery write. Once a runtime has
        // crossed an uncertainty boundary, no concurrent dispatch may reopen the lane while this
        // handoff is being retained. The recovery lease deliberately bypasses that fence so an
        // already-running quarantine can still account for the existing handoff.
        let _already_quarantined = begin_quarantine(&self.store)?;
        let mut store = try_lock_quarantine(&self.store).map_err(|error| {
            format!("cannot acquire worker recovery lease for unknown handoff: {error}")
        })?;
        let result = store
            .mark_worker_handoff_unknown(handoff_id)
            .map(|_| ())
            .map_err(|error| format!("cannot retain uncertain worker handoff: {error}"));
        if result.is_ok() {
            finish_quarantine(&self.store);
        }
        result
    }

    /// Returns the fixed startup failure until a protected platform transport is integrated. No
    /// request, credential, or synthetic peer is created on this path.
    fn transport_unavailable(&self) -> Result<(), String> {
        Err(String::from(MISSING_TRANSPORT_ERROR))
    }
}

/// Starts worker mode, persists its stopped boot, and fails closed until a native authenticated
/// listener is supplied by the platform-specific transport integration.
pub(crate) fn run(config: RuntimeConfig) -> Result<(), String> {
    let settings = WorkerSettings::from_environment(&config)?;
    let runtime = WorkerRuntime::open(settings)?;
    let result = runtime.transport_unavailable();
    let close = runtime.close();
    match close {
        Ok(()) => result,
        Err(error) => Err(format!("{MISSING_TRANSPORT_ERROR}; {error}")),
    }
}

fn command_error(error: WorkerCommandError) -> String {
    format!("worker command failed: {error}")
}

fn combine_failure(original: String, quarantine: Result<(), String>) -> String {
    match quarantine {
        Ok(()) => original,
        Err(error) => format!("{original}; failed to retain unknown worker handoff: {error}"),
    }
}

#[cfg(test)]
#[path = "runtime_v3_worker_runtime_tests.rs"]
mod tests;

#[path = "runtime_v3_worker_runtime_execution.rs"]
mod execution;
