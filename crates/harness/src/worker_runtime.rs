// SPDX-License-Identifier: MIT

//! Worker admission, response-before-start ordering, and capacity-one execution state.
//! Native authentication and executable gameplay adapters remain separate boundaries.

use crate::worker_handoff::{
    AuthenticatedWorkerRequest, DispatchReply, WorkerCommand, WorkerCommandAdmission,
    WorkerCommandConfig, WorkerCommandError, WorkerExecutionReservation, WorkerReply,
};
use crate::{ExecutionFingerprint, StoredWorkerHandoff, WorkerHandoffState, WorkerTuple};

use crate::worker_runtime_store::{
    SharedExecutionStore, begin_quarantine, finish_quarantine, try_lock, try_lock_close,
    try_lock_quarantine, try_lock_recovery,
};

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
    execution_taken: bool,
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
        self.execution_taken = false;
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
    execution_owner: std::sync::Arc<()>,
}

impl WorkerRuntime {
    /// Test and native-adapter constructor for an already-open worker-owned store.
    pub fn from_shared_store(
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
            execution_owner: std::sync::Arc::new(()),
        })
    }

    /// Handles an already-authenticated command.  The native endpoint must retain the original
    /// request for response encoding and call [`Self::finish_exchange`] only after its response
    /// write has completed.
    pub fn handle_authenticated(
        &mut self,
        authenticated: &AuthenticatedWorkerRequest,
    ) -> Result<WorkerExchange, String> {
        if authenticated.request().command() == crate::worker_handoff::WorkerCommand::Dispatch {
            return self.handle_dispatch(authenticated);
        }
        // Quarantine blocks new authority, not authenticated historical lookup
        // or a more restrictive operator intent. Running control still needs
        // the ordinary admission lease and can never clear this local fence.
        let recovery_safe = match authenticated.request().command() {
            WorkerCommand::Probe | WorkerCommand::Lookup | WorkerCommand::Acknowledge => true,
            WorkerCommand::SetControlMode => matches!(
                authenticated
                    .request()
                    .fields()
                    .get("mode")
                    .and_then(serde_json::Value::as_str),
                Some("paused" | "draining" | "stopped")
            ),
            WorkerCommand::Dispatch => false,
        };
        let mut store = if recovery_safe {
            try_lock_recovery(&self.store)?
        } else {
            try_lock(&self.store)?
        };
        let mut result = self
            .admission
            .handle_authenticated(&mut store, authenticated)
            .map_err(command_error)?;
        let reservation = result.take_reservation();
        let (mut reply, leftover) = result.into_parts();
        if let WorkerReply::Probe(probe) = &mut reply {
            probe.ready &= self.store.admission_open();
        }
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

    /// Store owner shared with the admitted execution adapter.
    pub fn store(&self) -> &SharedExecutionStore {
        &self.store
    }

    /// Owner-pinned execution fingerprint; never derived from a command frame.
    pub fn fingerprint(&self) -> &ExecutionFingerprint {
        &self.fingerprint
    }

    /// The currently occupied single execution lane.
    pub fn active_tuple(&self) -> Option<&WorkerTuple> {
        self.lane.tuple()
    }

    pub fn worker_boot_id(&self) -> &str {
        &self.worker_boot_id
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

#[path = "worker_runtime_completion.rs"]
mod completion;
#[path = "worker_runtime_execution.rs"]
mod execution;
pub use execution::{WorkerExecutionCompletion, WorkerExecutionTask};
#[cfg(test)]
#[path = "worker_runtime_control_tests.rs"]
mod control_tests;
#[cfg(test)]
#[path = "worker_runtime_tests.rs"]
pub(crate) mod tests;
