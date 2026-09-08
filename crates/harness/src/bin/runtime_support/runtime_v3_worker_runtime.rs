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
    ExecutionFingerprint, ExecutionLineage, ExecutionStore, ExecutionStoreConfig,
    StoredWorkerHandoff, WorkerHandoffState, WorkerTuple,
};

use super::super::config::RuntimeConfig;
use super::super::runtime_v3_settings::RuntimeV3Settings;
use super::super::worker_settings::WorkerSettings;
use super::worker_store::{SharedExecutionStore, share_store, try_lock};

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
    approved: sts2_harness::worker_handoff::ApprovedWorkerExecution,
    lineage: ExecutionLineage,
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
            settings.approved,
            settings.lineage,
            settings.fingerprint,
            settings.boot.worker_boot_id,
        )
    }

    /// Test and native-adapter constructor for an already-open worker-owned store.
    pub fn from_shared_store(
        store: SharedExecutionStore,
        command: WorkerCommandConfig,
        approved: sts2_harness::worker_handoff::ApprovedWorkerExecution,
        lineage: ExecutionLineage,
        fingerprint: ExecutionFingerprint,
        worker_boot_id: String,
    ) -> Result<Self, String> {
        let admission = WorkerCommandAdmission::new(command)
            .map_err(|error| format!("worker command configuration is invalid: {error}"))?;
        Ok(Self {
            store,
            admission,
            approved,
            lineage,
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
            .prepare_dispatch(&mut store, authenticated, self.approved.clone())
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
            self.retain_unknown(&handoff_id)?;
            return Ok(WorkerStartOutcome::Unknown { handoff_id });
        }
        let mut store = try_lock(&self.store)?;
        match reservation.start(&mut store) {
            Ok(running) => {
                if running.state != WorkerHandoffState::Running {
                    drop(store);
                    self.retain_unknown(&handoff_id)?;
                    return Err(String::from(
                        "worker reservation did not reach the running state",
                    ));
                }
                Ok(WorkerStartOutcome::Started(Box::new(running)))
            }
            Err(error) => {
                drop(store);
                self.retain_unknown(&handoff_id)?;
                Err(format!("worker reservation could not start: {error}"))
            }
        }
    }

    fn retain_unknown(&self, handoff_id: &str) -> Result<(), String> {
        let mut store = try_lock(&self.store)?;
        store
            .mark_worker_handoff_unknown(handoff_id)
            .map(|_| ())
            .map_err(|_| String::from("cannot retain uncertain worker handoff"))
    }

    /// Runs the already-started handoff through the existing admitted runtime path.  This method
    /// is intentionally reachable to a future authenticated listener, but production `run`
    /// below does not call it until a real protected native transport is present.
    pub fn execute_started(
        &mut self,
        running: StoredWorkerHandoff,
        config: RuntimeConfig,
        settings: RuntimeV3Settings,
    ) -> Result<(), String> {
        let handoff_id = running.tuple.handoff_id.clone();
        let result = self.execute_started_inner(running, config, settings);
        if result.is_err() {
            // The execution side may already have projected uncertainty. This idempotent fence
            // also covers launch/configuration failures before it could enter that path.
            let _ = self.retain_unknown(&handoff_id);
        }
        result
    }

    fn execute_started_inner(
        &mut self,
        running: StoredWorkerHandoff,
        config: RuntimeConfig,
        settings: RuntimeV3Settings,
    ) -> Result<(), String> {
        let Some(active) = self.lane.tuple() else {
            return Err(String::from("worker execution lane has no active handoff"));
        };
        if running.tuple != *active || running.worker_boot_id != self.worker_boot_id {
            return Err(String::from(
                "started worker handoff does not match the active worker lane",
            ));
        }
        let launch_options = super::launch_options::RuntimeV3LaunchOptions::from_environment()?;
        let telemetry_context = super::super::runtime_v3_telemetry::TelemetryContext::new(
            super::super::runtime_v3_telemetry::TelemetryContextLineage {
                run_id: &config.run_id,
                episode_id: &config.episode_id,
                trajectory_id: &config.trajectory_id,
                trace_id: &config.trace_id,
            },
            &config.instance_id,
            &config.session_id,
            &config.runtime_profile,
            &settings.exo.revision,
        )?;
        let telemetry =
            super::super::runtime_v3_telemetry::RuntimeV3Telemetry::new(telemetry_context);
        let telemetry_handle = telemetry.handle();
        let _ = telemetry_handle.run_started();
        let (durable, state) = super::durable::DurableHandle::from_admitted_shared_store(
            self.store.clone(),
            &running,
            &config,
            &settings,
            self.lineage.clone(),
            self.fingerprint.clone(),
        )?;
        let result = super::execution::run(
            config,
            settings,
            durable,
            state,
            launch_options,
            telemetry_handle,
            telemetry,
        );
        if result.is_ok() {
            self.release_completed(&running.tuple.handoff_id)?;
        }
        result
    }

    /// Releases the capacity-one lane only after the same durable handoff has a projected
    /// terminal receipt. Unknown, admitted, and running rows remain lookup-only.
    pub fn release_completed(&mut self, handoff_id: &str) -> Result<(), String> {
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
            sts2_harness::WorkerLookup::Known(handoff) => *handoff,
            sts2_harness::WorkerLookup::Unknown { .. } => {
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
        let mut store = try_lock(&self.store)?;
        store
            .close()
            .map_err(|_| String::from("cannot close worker execution store"))
    }

    pub fn worker_boot_id(&self) -> &str {
        &self.worker_boot_id
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

#[cfg(test)]
#[path = "runtime_v3_worker_runtime_tests.rs"]
mod tests;
