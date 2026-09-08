// SPDX-License-Identifier: MIT

//! Private types shared by the worker-command admission policy and its tests.

use std::fmt;

use crate::execution::{
    ExecutionFingerprint, ExecutionLineage, ExecutionStore, StoredWorkerHandoff,
    WorkerAdmissionContext, WorkerExecutionPermit, WorkerTuple,
};

use super::super::{DispatchReply, WorkerReply, WorkerRequest};

#[path = "worker_command_authentication.rs"]
mod authentication;
pub use authentication::{AuthenticatedWorkerRequest, WorkerCapability};

/// Immutable values pinned by the owner-approved worker launch record.
///
/// The fields intentionally remain private.  A transport adapter may construct this value from
/// its already-authorized launch configuration, but it cannot mutate the binding after admission
/// has been created.
pub struct WorkerCommandConfig {
    pub(in crate::worker_handoff) deployment_id: String,
    pub(in crate::worker_handoff) worker_owner_id: String,
    pub(in crate::worker_handoff) worker_profile_digest: String,
    pub(in crate::worker_handoff) release_digest: String,
    pub(in crate::worker_handoff) config_digest: String,
    pub(in crate::worker_handoff) worker_boot_id: String,
    pub(in crate::worker_handoff) watchdog_boot_id: String,
}

impl WorkerCommandConfig {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        deployment_id: impl Into<String>,
        worker_owner_id: impl Into<String>,
        worker_profile_digest: impl Into<String>,
        release_digest: impl Into<String>,
        config_digest: impl Into<String>,
        worker_boot_id: impl Into<String>,
        watchdog_boot_id: impl Into<String>,
    ) -> Result<Self, WorkerCommandError> {
        let config = Self {
            deployment_id: deployment_id.into(),
            worker_owner_id: worker_owner_id.into(),
            worker_profile_digest: worker_profile_digest.into(),
            release_digest: release_digest.into(),
            config_digest: config_digest.into(),
            worker_boot_id: worker_boot_id.into(),
            watchdog_boot_id: watchdog_boot_id.into(),
        };
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), WorkerCommandError> {
        if !identity(&self.deployment_id)
            || !identity(&self.worker_owner_id)
            || !digest(&self.worker_profile_digest)
            || !digest(&self.release_digest)
            || !digest(&self.config_digest)
            || !uuid4(&self.worker_boot_id)
            || !uuid4(&self.watchdog_boot_id)
            || self.worker_boot_id == self.watchdog_boot_id
        {
            return Err(WorkerCommandError::InvalidBinding);
        }
        Ok(())
    }
}

/// Caller-supplied execution material from an approved runtime launch record.
///
/// The command frame cannot create this value. Its fingerprint is intentionally retained here so
/// preparation can bind the frame to the approved seed/build/state/config/provider values instead
/// of deriving any of them from remote JSON.
#[derive(Clone)]
pub struct ApprovedWorkerExecution {
    pub(in crate::worker_handoff) lineage: ExecutionLineage,
    pub(in crate::worker_handoff) fingerprint: ExecutionFingerprint,
    pub(in crate::worker_handoff) job_id: String,
    pub(in crate::worker_handoff) attempt_number: u64,
}

impl ApprovedWorkerExecution {
    pub fn new(
        lineage: ExecutionLineage,
        fingerprint: ExecutionFingerprint,
        job_id: impl Into<String>,
        attempt_number: u64,
    ) -> Result<Self, WorkerCommandError> {
        let execution = Self {
            lineage,
            fingerprint,
            job_id: job_id.into(),
            attempt_number,
        };
        execution
            .lineage
            .validate()
            .map_err(|_| WorkerCommandError::InvalidBinding)?;
        execution
            .fingerprint
            .validate()
            .map_err(|_| WorkerCommandError::InvalidBinding)?;
        if !existing_id(&execution.job_id)
            || execution.attempt_number == 0
            || execution.attempt_number > crate::execution::WORKER_MAX_ATTEMPT_NUMBER
        {
            return Err(WorkerCommandError::InvalidBinding);
        }
        Ok(execution)
    }
}

/// A completed, idempotent preparation. It owns the exact tuple/context used by the atomic store
/// call; it is not itself execution authority and has no public constructor.
pub struct WorkerDispatchPreparation {
    pub(in crate::worker_handoff) tuple: WorkerTuple,
    pub(in crate::worker_handoff) context: WorkerAdmissionContext,
}

pub struct WorkerCommandResult {
    pub(in crate::worker_handoff) reply: WorkerReply,
    reservation: Option<WorkerExecutionReservation>,
}

impl WorkerCommandResult {
    pub(in crate::worker_handoff) fn reply(reply: WorkerReply) -> Self {
        Self {
            reply,
            reservation: None,
        }
    }

    pub(in crate::worker_handoff) fn accepted(
        preparation: WorkerDispatchPreparation,
        permit: WorkerExecutionPermit,
    ) -> Self {
        Self {
            reply: WorkerReply::Dispatch(DispatchReply::Accepted),
            reservation: Some(WorkerExecutionReservation {
                tuple: preparation.tuple,
                context: preparation.context,
                permit,
            }),
        }
    }

    /// Moves the one-time execution reservation to the runtime bridge. Dropping the result
    /// without taking it intentionally leaves the durable handoff admitted for reconciliation.
    pub fn take_reservation(&mut self) -> Option<WorkerExecutionReservation> {
        self.reservation.take()
    }

    pub fn into_parts(self) -> (WorkerReply, Option<WorkerExecutionReservation>) {
        (self.reply, self.reservation)
    }
}

/// A fresh dispatch winner owns the store-issued permit and the exact admission context/tuple.
/// The runtime bridge must consume this value after the response is written and immediately
/// before provider/episode execution. There is no constructor from a handoff row or string.
pub struct WorkerExecutionReservation {
    tuple: WorkerTuple,
    context: WorkerAdmissionContext,
    permit: WorkerExecutionPermit,
}

impl WorkerExecutionReservation {
    pub fn tuple(&self) -> &WorkerTuple {
        &self.tuple
    }

    /// Consumes the opaque permit and performs the durable current-control fence immediately
    /// before execution. Calling this twice is impossible because the reservation is consumed.
    pub fn start(
        self,
        store: &mut ExecutionStore,
    ) -> Result<StoredWorkerHandoff, WorkerCommandError> {
        store
            .mark_worker_handoff_running(self.permit, &self.context)
            .map_err(|error| match error {
                crate::execution::ExecutionStoreError::Conflict => {
                    WorkerCommandError::ReservationMismatch
                }
                _ => WorkerCommandError::Store,
            })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerCommandError {
    InvalidRequest,
    InvalidBinding,
    Unauthorized,
    IdentityMismatch,
    Store,
    ReservationMismatch,
}

impl fmt::Display for WorkerCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidRequest => "invalid worker command request",
            Self::InvalidBinding => "worker command binding is invalid",
            Self::Unauthorized => "worker command capability is unauthorized",
            Self::IdentityMismatch => "worker command identity does not match configuration",
            Self::Store => "worker command durable store operation failed",
            Self::ReservationMismatch => {
                "worker execution reservation does not match current control"
            }
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for WorkerCommandError {}

impl From<crate::execution::ExecutionStoreError> for WorkerCommandError {
    fn from(_: crate::execution::ExecutionStoreError) -> Self {
        Self::Store
    }
}

pub(in crate::worker_handoff) fn tuple(
    request: &WorkerRequest,
) -> Result<WorkerTuple, WorkerCommandError> {
    let fields = request.fields();
    WorkerTuple::new(
        text(fields, "handoff_id")?,
        text(fields, "deployment_id")?,
        text(fields, "job_id")?,
        text(fields, "attempt_id")?,
        number(fields, "attempt_number")?,
        text(fields, "worker_owner_id")?,
        text(fields, "worker_profile_digest")?,
        text(fields, "run_id")?,
        text(fields, "episode_id")?,
        text(fields, "trajectory_id")?,
        text(fields, "payload_digest")?,
    )
    .map_err(|_| WorkerCommandError::InvalidRequest)
}

pub(in crate::worker_handoff) fn context(
    request: &WorkerRequest,
) -> Result<WorkerAdmissionContext, WorkerCommandError> {
    let fields = request.fields();
    WorkerAdmissionContext::new(
        text(fields, "watchdog_boot_id")?,
        text(fields, "worker_boot_id")?,
        number(fields, "mode_sequence")?,
    )
    .map_err(|_| WorkerCommandError::InvalidRequest)
}

pub(in crate::worker_handoff) fn text<'a>(
    fields: &'a serde_json::Map<String, serde_json::Value>,
    name: &str,
) -> Result<&'a str, WorkerCommandError> {
    fields
        .get(name)
        .and_then(serde_json::Value::as_str)
        .ok_or(WorkerCommandError::InvalidRequest)
}

pub(in crate::worker_handoff) fn number(
    fields: &serde_json::Map<String, serde_json::Value>,
    name: &str,
) -> Result<u64, WorkerCommandError> {
    fields
        .get(name)
        .and_then(serde_json::Value::as_u64)
        .ok_or(WorkerCommandError::InvalidRequest)
}

fn existing_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}

pub(in crate::worker_handoff) fn identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

pub(in crate::worker_handoff) fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(in crate::worker_handoff) fn uuid4(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| {
        id.get_version_num() == 4
            && id.get_variant() == uuid::Variant::RFC4122
            && id.to_string() == value
    })
}
