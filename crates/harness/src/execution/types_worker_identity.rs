// SPDX-License-Identifier: MIT

use super::super::core::{ExecutionLineage, valid_digest};
use super::super::error::ExecutionStoreError;

pub const WORKER_HANDOFF_CONTRACT: &str = "ascension-watchdog-worker-handoff-v1";
pub const WORKER_HANDOFF_SCHEMA_DIGEST: &str =
    "bb13d15f6c0e4b8d0f58f7391fe4ba319ebc57a0a09effc06d73ea718bbff4cf";
pub const WORKER_EMPTY_PARAMETERS_DIGEST: &str =
    "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a";
pub const WORKER_MAX_ATTEMPT_NUMBER: u64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerTuple {
    pub handoff_id: String,
    pub deployment_id: String,
    pub job_id: String,
    pub attempt_id: String,
    pub attempt_number: u64,
    pub worker_owner_id: String,
    pub worker_profile_digest: String,
    pub run_id: String,
    pub episode_id: String,
    pub trajectory_id: String,
    pub payload_digest: String,
}

impl WorkerTuple {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        handoff_id: impl Into<String>,
        deployment_id: impl Into<String>,
        job_id: impl Into<String>,
        attempt_id: impl Into<String>,
        attempt_number: u64,
        worker_owner_id: impl Into<String>,
        worker_profile_digest: impl Into<String>,
        run_id: impl Into<String>,
        episode_id: impl Into<String>,
        trajectory_id: impl Into<String>,
        payload_digest: impl Into<String>,
    ) -> Result<Self, ExecutionStoreError> {
        let tuple = Self {
            handoff_id: handoff_id.into(),
            deployment_id: deployment_id.into(),
            job_id: job_id.into(),
            attempt_id: attempt_id.into(),
            attempt_number,
            worker_owner_id: worker_owner_id.into(),
            worker_profile_digest: worker_profile_digest.into(),
            run_id: run_id.into(),
            episode_id: episode_id.into(),
            trajectory_id: trajectory_id.into(),
            payload_digest: payload_digest.into(),
        };
        tuple.validate()?;
        Ok(tuple)
    }

    pub fn validate(&self) -> Result<(), ExecutionStoreError> {
        if !valid_worker_uuid4(&self.handoff_id)
            || !valid_worker_identity(&self.deployment_id)
            || !valid_worker_existing_id(&self.job_id)
            || !valid_worker_existing_id(&self.attempt_id)
            || !valid_worker_identity(&self.worker_owner_id)
            || !valid_digest(&self.worker_profile_digest)
            || !valid_digest(&self.payload_digest)
            || !valid_worker_uuid4(&self.run_id)
            || !valid_worker_uuid4(&self.episode_id)
            || !valid_worker_uuid4(&self.trajectory_id)
            || self.payload_digest != WORKER_EMPTY_PARAMETERS_DIGEST
            || self.attempt_number == 0
            || self.attempt_number > WORKER_MAX_ATTEMPT_NUMBER
        {
            return Err(ExecutionStoreError::InvalidJob);
        }
        let lineage = self.lineage()?;
        if self.handoff_id == self.run_id
            || self.handoff_id == self.episode_id
            || self.handoff_id == self.trajectory_id
            || self.run_id == self.episode_id
            || self.run_id == self.trajectory_id
            || self.episode_id == self.trajectory_id
        {
            return Err(ExecutionStoreError::InvalidIdentity);
        }
        lineage.validate()
    }

    pub(crate) fn lineage(&self) -> Result<ExecutionLineage, ExecutionStoreError> {
        ExecutionLineage::new(
            self.run_id.clone(),
            self.episode_id.clone(),
            self.attempt_id.clone(),
            self.trajectory_id.clone(),
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerBoot {
    pub deployment_id: String,
    pub worker_owner_id: String,
    pub worker_profile_digest: String,
    pub worker_boot_id: String,
}

impl WorkerBoot {
    pub fn new(
        deployment_id: impl Into<String>,
        worker_owner_id: impl Into<String>,
        worker_profile_digest: impl Into<String>,
        worker_boot_id: impl Into<String>,
    ) -> Result<Self, ExecutionStoreError> {
        let boot = Self {
            deployment_id: deployment_id.into(),
            worker_owner_id: worker_owner_id.into(),
            worker_profile_digest: worker_profile_digest.into(),
            worker_boot_id: worker_boot_id.into(),
        };
        boot.validate()?;
        Ok(boot)
    }

    pub fn validate(&self) -> Result<(), ExecutionStoreError> {
        if !valid_worker_identity(&self.deployment_id)
            || !valid_worker_identity(&self.worker_owner_id)
            || !valid_worker_uuid4(&self.worker_boot_id)
            || !valid_digest(&self.worker_profile_digest)
        {
            return Err(ExecutionStoreError::InvalidIdentity);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerAdmissionContext {
    pub watchdog_boot_id: String,
    pub worker_boot_id: String,
    pub mode_sequence: u64,
}

impl WorkerAdmissionContext {
    pub fn new(
        watchdog_boot_id: impl Into<String>,
        worker_boot_id: impl Into<String>,
        mode_sequence: u64,
    ) -> Result<Self, ExecutionStoreError> {
        let context = Self {
            watchdog_boot_id: watchdog_boot_id.into(),
            worker_boot_id: worker_boot_id.into(),
            mode_sequence,
        };
        context.validate()?;
        Ok(context)
    }

    pub fn validate(&self) -> Result<(), ExecutionStoreError> {
        if !valid_worker_uuid4(&self.watchdog_boot_id)
            || !valid_worker_uuid4(&self.worker_boot_id)
            || self.mode_sequence == 0
            || self.mode_sequence > WORKER_MAX_ATTEMPT_NUMBER
        {
            return Err(ExecutionStoreError::InvalidIdentity);
        }
        Ok(())
    }
}

pub(crate) fn valid_worker_uuid4(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && [8, 13, 18, 23]
            .into_iter()
            .all(|index| bytes[index] == b'-')
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => true,
            14 => *byte == b'4',
            19 => matches!(*byte, b'8' | b'9' | b'a' | b'b'),
            _ => byte.is_ascii_digit() || (b'a'..=b'f').contains(byte),
        })
}

pub(crate) fn valid_worker_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

pub(crate) fn valid_worker_existing_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}
