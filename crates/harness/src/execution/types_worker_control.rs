// SPDX-License-Identifier: MIT

use super::super::core::valid_digest;
use super::super::error::ExecutionStoreError;
use super::identity::{WORKER_MAX_ATTEMPT_NUMBER, valid_worker_identity, valid_worker_uuid4};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerControlRequest {
    pub deployment_id: String,
    pub worker_owner_id: String,
    pub worker_profile_digest: String,
    pub watchdog_boot_id: String,
    pub worker_boot_id: String,
    pub mode: WorkerControlMode,
    pub mode_sequence: u64,
}

impl WorkerControlRequest {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        deployment_id: impl Into<String>,
        worker_owner_id: impl Into<String>,
        worker_profile_digest: impl Into<String>,
        watchdog_boot_id: impl Into<String>,
        worker_boot_id: impl Into<String>,
        mode: WorkerControlMode,
        mode_sequence: u64,
    ) -> Result<Self, ExecutionStoreError> {
        let request = Self {
            deployment_id: deployment_id.into(),
            worker_owner_id: worker_owner_id.into(),
            worker_profile_digest: worker_profile_digest.into(),
            watchdog_boot_id: watchdog_boot_id.into(),
            worker_boot_id: worker_boot_id.into(),
            mode,
            mode_sequence,
        };
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<(), ExecutionStoreError> {
        if !valid_worker_identity(&self.deployment_id)
            || !valid_worker_identity(&self.worker_owner_id)
            || !valid_digest(&self.worker_profile_digest)
            || !valid_worker_uuid4(&self.watchdog_boot_id)
            || !valid_worker_uuid4(&self.worker_boot_id)
            || self.mode_sequence == 0
            || self.mode_sequence > WORKER_MAX_ATTEMPT_NUMBER
        {
            return Err(ExecutionStoreError::InvalidIdentity);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerControlMode {
    Running,
    Paused,
    Draining,
    Stopped,
}

impl WorkerControlMode {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Draining => "draining",
            Self::Stopped => "stopped",
        }
    }

    pub(crate) fn from_str(value: &str) -> Option<Self> {
        Some(match value {
            "running" => Self::Running,
            "paused" => Self::Paused,
            "draining" => Self::Draining,
            "stopped" => Self::Stopped,
            _ => return None,
        })
    }

    #[must_use]
    pub const fn admits(self) -> bool {
        matches!(self, Self::Running)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerControlState {
    pub deployment_id: String,
    pub worker_owner_id: String,
    pub worker_profile_digest: String,
    pub worker_boot_id: String,
    pub watchdog_boot_id: Option<String>,
    pub mode: WorkerControlMode,
    pub mode_sequence: u64,
    pub generation: u64,
    pub authenticated: bool,
    pub admitting: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerOwnerProof {
    marker: String,
}

impl WorkerOwnerProof {
    /// The transport/server supplies this only after authenticating the configured owner. The
    /// execution store records the fact that the boundary was crossed; it does not authenticate
    /// OS peers or inspect credentials itself.
    pub fn new(marker: impl Into<String>) -> Result<Self, ExecutionStoreError> {
        let marker = marker.into();
        if marker.is_empty() || marker.len() > 512 || marker.chars().any(char::is_control) {
            return Err(ExecutionStoreError::InvalidIdentity);
        }
        Ok(Self { marker })
    }

    pub(crate) fn is_present(&self) -> bool {
        !self.marker.is_empty()
    }
}
