// SPDX-License-Identifier: MIT

//! Authenticated worker-command admission policy.
//!
//! This module is private to the worker-handoff endpoint. Transport code owns peer
//! authentication and supplies the opaque authorization facts; this module owns command
//! capability, immutable binding, and store ordering. It does not execute a worker operation
//! while handling a connection.

#[path = "worker_command_operations.rs"]
mod operations;
#[path = "worker_command_support.rs"]
mod support;

use crate::execution::{
    AttemptState, ExecutionFingerprint, ExecutionLineage, ExecutionStore, WorkerAdmissionOutcome,
    WorkerControlMode, WorkerHandoffState,
};

use super::{WorkerCommand, WorkerReply};
pub use support::{
    ApprovedWorkerExecution, AuthenticatedWorkerRequest, WorkerCapability, WorkerCommandConfig,
    WorkerCommandError, WorkerCommandResult, WorkerDispatchPreparation, WorkerExecutionReservation,
};
use support::{context, text, tuple};

/// Private command admission state bound to one owner-approved worker launch.
/// Safe command-admission adapter used by an authenticated worker endpoint.
///
/// The adapter owns no transport and does not authenticate callers.  The caller must construct
/// [`AuthenticatedWorkerRequest`] only after its transport-specific peer checks have succeeded.
pub struct WorkerCommandAdmission {
    config: WorkerCommandConfig,
}

impl WorkerCommandAdmission {
    pub fn new(config: WorkerCommandConfig) -> Result<Self, WorkerCommandError> {
        config.validate()?;
        Ok(Self { config })
    }

    /// Handles historical and control commands after transport authentication. Dispatch is
    /// rejected here because it requires the separate approved-preparation path.
    pub fn handle_authenticated(
        &self,
        store: &mut ExecutionStore,
        authenticated: &AuthenticatedWorkerRequest,
    ) -> Result<WorkerCommandResult, WorkerCommandError> {
        self.validate_request(authenticated)?;
        match authenticated.request.command() {
            WorkerCommand::Probe => self.probe(store),
            WorkerCommand::Dispatch => self.dispatch_reply(authenticated),
            WorkerCommand::Lookup => self.lookup(store, &authenticated.request),
            WorkerCommand::Acknowledge => self.acknowledge(store, &authenticated.request),
            WorkerCommand::SetControlMode => self.set_control(store, authenticated),
        }
    }

    /// Performs only the approved runtime preparation needed before atomic handoff admission.
    ///
    /// `approved` must be built from owner-controlled runtime configuration. The request cannot
    /// supply its fingerprint, seed, provider identity, or executable configuration. Missing
    /// episode/job rows are created only from that approved material; retained rows are matched
    /// exactly, and a fresh admission requires an active episode plus an admitted job.
    pub fn prepare_dispatch(
        &self,
        store: &mut ExecutionStore,
        authenticated: &AuthenticatedWorkerRequest,
        approved: ApprovedWorkerExecution,
    ) -> Result<WorkerDispatchPreparation, WorkerCommandError> {
        self.validate_request(authenticated)?;
        if authenticated.request.command() != WorkerCommand::Dispatch {
            return Err(WorkerCommandError::InvalidRequest);
        }
        let tuple = tuple(&authenticated.request)?;
        let context = context(&authenticated.request)?;
        self.validate_approved(&tuple, &approved)?;
        self.require_current_control(store, &tuple, &context)?;

        let existing = store.worker_handoff(&tuple.handoff_id)?;
        if let Some(existing) = &existing {
            if existing.tuple != tuple {
                return Err(WorkerCommandError::InvalidBinding);
            }
            self.validate_existing_records(store, &tuple, &approved, false)?;
        } else {
            store.start_episode(&approved.lineage, &approved.fingerprint)?;
            let job = store.admit_job(
                &approved.job_id,
                &approved.lineage.episode_id,
                &tuple.payload_digest,
            )?;
            if job.state != crate::execution::JobState::Admitted {
                return Err(WorkerCommandError::ReservationMismatch);
            }
            self.validate_existing_records(store, &tuple, &approved, true)?;
        }
        Ok(WorkerDispatchPreparation { tuple, context })
    }

    /// Maps the fresh identity tuple in an authenticated dispatch to the harness-owned execution
    /// policy, then performs the ordinary approved preparation. Job, attempt and lineage IDs are
    /// scheduling facts from the frozen request; fingerprint fields remain owner-pinned harness
    /// state and are never accepted from the request.
    pub fn prepare_dispatch_from_authenticated(
        &self,
        store: &mut ExecutionStore,
        authenticated: &AuthenticatedWorkerRequest,
        fingerprint: ExecutionFingerprint,
    ) -> Result<WorkerDispatchPreparation, WorkerCommandError> {
        self.validate_request(authenticated)?;
        if authenticated.request.command() != WorkerCommand::Dispatch {
            return Err(WorkerCommandError::InvalidRequest);
        }
        let tuple = tuple(&authenticated.request)?;
        let context = context(&authenticated.request)?;
        self.require_current_control(store, &tuple, &context)?;
        let lineage = ExecutionLineage::new(
            tuple.run_id.clone(),
            tuple.episode_id.clone(),
            tuple.attempt_id.clone(),
            tuple.trajectory_id.clone(),
        )
        .map_err(|_| WorkerCommandError::InvalidBinding)?;
        let approved = ApprovedWorkerExecution::new(
            lineage,
            fingerprint,
            tuple.job_id.clone(),
            tuple.attempt_number,
        )?;
        self.prepare_dispatch(store, authenticated, approved)
    }

    /// Revalidates a prepared request and atomically admits only a fresh execution winner.
    /// Duplicate observations return a retained status without an execution reservation.
    pub fn admit_prepared_dispatch(
        &self,
        store: &mut ExecutionStore,
        authenticated: &AuthenticatedWorkerRequest,
        preparation: WorkerDispatchPreparation,
    ) -> Result<WorkerCommandResult, WorkerCommandError> {
        self.validate_request(authenticated)?;
        if authenticated.request.command() != WorkerCommand::Dispatch
            || tuple(&authenticated.request)? != preparation.tuple
            || context(&authenticated.request)? != preparation.context
        {
            return Err(WorkerCommandError::ReservationMismatch);
        }
        match store.admit_worker_handoff(&preparation.tuple, &preparation.context)? {
            WorkerAdmissionOutcome::Acquired { handoff, permit } => {
                if handoff.tuple != preparation.tuple
                    || handoff.state != WorkerHandoffState::Admitted
                {
                    return Err(WorkerCommandError::Store);
                }
                Ok(WorkerCommandResult::accepted(preparation, permit))
            }
            WorkerAdmissionOutcome::Duplicate(handoff) => Ok(WorkerCommandResult::reply(
                WorkerReply::Dispatch(self.duplicate_dispatch_reply(handoff)?),
            )),
        }
    }

    fn validate_existing_records(
        &self,
        store: &mut ExecutionStore,
        tuple: &crate::execution::WorkerTuple,
        approved: &ApprovedWorkerExecution,
        require_admission: bool,
    ) -> Result<(), WorkerCommandError> {
        let episode = store.load_episode(&approved.lineage.episode_id)?;
        if episode.lineage != approved.lineage || episode.fingerprint != approved.fingerprint {
            return Err(WorkerCommandError::ReservationMismatch);
        }
        let job = store.job(&approved.job_id)?;
        if job.episode_id != tuple.episode_id || job.payload_digest != tuple.payload_digest {
            return Err(WorkerCommandError::ReservationMismatch);
        }
        if require_admission
            && (episode.state != AttemptState::Active
                || job.state != crate::execution::JobState::Admitted)
        {
            return Err(WorkerCommandError::ReservationMismatch);
        }
        Ok(())
    }

    fn validate_request(
        &self,
        authenticated: &AuthenticatedWorkerRequest,
    ) -> Result<(), WorkerCommandError> {
        if authenticated.capability.command() != authenticated.request.command() {
            return Err(WorkerCommandError::Unauthorized);
        }
        let fields = authenticated.request.fields();
        if text(fields, "watchdog_boot_id")? != self.config.watchdog_boot_id {
            return Err(WorkerCommandError::IdentityMismatch);
        }
        if authenticated.request.command() != WorkerCommand::Probe
            && text(fields, "worker_boot_id")? != self.config.worker_boot_id
        {
            return Err(WorkerCommandError::IdentityMismatch);
        }
        match authenticated.request.command() {
            WorkerCommand::Dispatch | WorkerCommand::Lookup | WorkerCommand::Acknowledge => {
                let tuple = tuple(&authenticated.request)?;
                if tuple.deployment_id != self.config.deployment_id
                    || tuple.worker_owner_id != self.config.worker_owner_id
                    || tuple.worker_profile_digest != self.config.worker_profile_digest
                {
                    return Err(WorkerCommandError::IdentityMismatch);
                }
            }
            WorkerCommand::SetControlMode => {
                if text(fields, "deployment_id")? != self.config.deployment_id
                    || text(fields, "worker_owner_id")? != self.config.worker_owner_id
                    || text(fields, "worker_profile_digest")? != self.config.worker_profile_digest
                {
                    return Err(WorkerCommandError::IdentityMismatch);
                }
            }
            WorkerCommand::Probe => {}
        }
        Ok(())
    }

    fn validate_approved(
        &self,
        tuple: &crate::execution::WorkerTuple,
        approved: &ApprovedWorkerExecution,
    ) -> Result<(), WorkerCommandError> {
        if tuple.deployment_id != self.config.deployment_id
            || tuple.worker_owner_id != self.config.worker_owner_id
            || tuple.worker_profile_digest != self.config.worker_profile_digest
            || tuple.job_id != approved.job_id
            || tuple.attempt_number != approved.attempt_number
            || approved.lineage.run_id != tuple.run_id
            || approved.lineage.episode_id != tuple.episode_id
            || approved.lineage.attempt_id != tuple.attempt_id
            || approved.lineage.trajectory_id != tuple.trajectory_id
            || approved.fingerprint.build_digest != self.config.release_digest
            || approved.fingerprint.config_digest != self.config.config_digest
        {
            return Err(WorkerCommandError::InvalidBinding);
        }
        Ok(())
    }

    fn require_current_control(
        &self,
        store: &ExecutionStore,
        tuple: &crate::execution::WorkerTuple,
        context: &crate::execution::WorkerAdmissionContext,
    ) -> Result<(), WorkerCommandError> {
        let Some(control) = store.worker_control()? else {
            return Err(WorkerCommandError::ReservationMismatch);
        };
        let matches = control.deployment_id == tuple.deployment_id
            && control.worker_owner_id == tuple.worker_owner_id
            && control.worker_profile_digest == tuple.worker_profile_digest
            && control.worker_boot_id == context.worker_boot_id
            && control.watchdog_boot_id.as_deref() == Some(context.watchdog_boot_id.as_str())
            && control.mode == WorkerControlMode::Running
            && control.mode_sequence == context.mode_sequence
            && control.authenticated
            && control.admitting;
        if matches {
            Ok(())
        } else {
            Err(WorkerCommandError::ReservationMismatch)
        }
    }
}

#[cfg(test)]
#[path = "worker_command_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "worker_command_mapping_tests.rs"]
mod mapping_tests;
