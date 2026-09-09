// SPDX-License-Identifier: MIT

//! Durable runtime-v3 state and its admission/evidence boundary around the gameplay MCP transport.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use sts2_harness::{
    Checkpoint, ExecutionFingerprint, ExecutionLineage, ExecutionStore, ExecutionStoreConfig,
    RECOVERY_SCHEMA_DIGEST, ResumeState,
};

use super::super::config::RuntimeConfig;
use super::super::runtime_v3_settings::RuntimeV3Settings;
#[cfg(test)]
use super::worker_store::snapshot;
use super::worker_store::{SharedExecutionStore, share_store, try_lock, try_lock_close};
use super::workflow_binding::WorkflowBinding;

const DEFAULT_STORE_PATH: &str = "harness-execution.sqlite3";
const PROVIDER_RESERVATION_UNITS: u64 = 1;

#[path = "runtime_v3_durable_admitted.rs"]
mod admitted;
#[path = "runtime_v3_durable_checkpoints.rs"]
mod checkpoints;
#[path = "runtime_v3_durable_completion.rs"]
mod completion;
#[path = "runtime_v3_durable_identity.rs"]
mod identity;
#[path = "runtime_v3_durable_operations.rs"]
mod operations;
#[path = "runtime_v3_durable_quarantine.rs"]
mod quarantine;
#[path = "runtime_v3_durable_support.rs"]
mod support;
#[path = "runtime_v3_durable_worker_fence.rs"]
mod worker_fence;

pub(super) use operations::{OperationCatalogEvidence, OperationIntentEvidence};

#[cfg(target_os = "linux")]
pub(super) use admitted::validate_worker_configuration;
use support::{config_digest_with_binding, optional_env, sha256_bytes};

/// A cloneable handle backed by the one worker-owned SQLite connection.
///
/// Each database lease is kept short. The handle's checkpoint metadata remains worker-local; the
/// `SharedExecutionStore` is the only value shared with control/probe/lookup paths.
#[derive(Clone)]
pub(super) struct DurableHandle {
    store: SharedExecutionStore,
    lineage: ExecutionLineage,
    fingerprint: ExecutionFingerprint,
    model_revision: String,
    config_digest: String,
    next_checkpoint: Rc<RefCell<u64>>,
    resume_boundary: Rc<RefCell<Option<Checkpoint>>>,
    workflow_binding: WorkflowBinding,
    owns_store: bool,
    worker_handoff: Option<Box<sts2_harness::StoredWorkerHandoff>>,
}

impl DurableHandle {
    #[allow(dead_code)]
    pub(super) fn open(
        config: &RuntimeConfig,
        settings: &RuntimeV3Settings,
        resume_requested: bool,
    ) -> Result<(Self, ResumeState), String> {
        Self::open_with_binding(
            config,
            settings,
            WorkflowBinding::default_full_episode(),
            resume_requested,
        )
    }

    pub(super) fn open_with_binding(
        config: &RuntimeConfig,
        settings: &RuntimeV3Settings,
        workflow_binding: WorkflowBinding,
        resume_requested: bool,
    ) -> Result<(Self, ResumeState), String> {
        let attempt_id = optional_env("STS2_ATTEMPT_ID")?
            .unwrap_or_else(|| format!("attempt-{}", config.episode_id));
        let lineage = ExecutionLineage::new(
            config.run_id.clone(),
            config.episode_id.clone(),
            attempt_id,
            config.trajectory_id.clone(),
        )
        .map_err(|error| format!("runtime-v3 execution lineage is invalid: {error}"))?;
        let fingerprint = support::fingerprint_with_binding(
            config,
            settings,
            &workflow_binding,
            resume_requested,
        )?;
        let path = optional_env("STS2_EXECUTION_STORE_PATH")?
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_STORE_PATH));
        let store_config = ExecutionStoreConfig::new(path).with_approved_recovery_schema();
        if store_config.recovery_schema_digest.as_deref() != Some(RECOVERY_SCHEMA_DIGEST) {
            return Err(String::from(
                "runtime-v3 execution store is not pinned to the approved recovery schema",
            ));
        }
        let mut store = ExecutionStore::open(store_config)
            .map_err(|error| format!("cannot open runtime-v3 execution store: {error}"))?;
        let (state, stored_episode) = {
            // Keep admission, the stored identity, resume state, and checkpoint on one shared
            // store lease. A control/probe path must not advance the episode between these reads.
            let existing = store
                .resume_episode(&lineage.episode_id, &fingerprint)
                .map_err(|error| format!("cannot inspect runtime-v3 execution state: {error}"))?;
            if !matches!(existing, ResumeState::New) && !resume_requested {
                return Err(String::from(
                    "durable runtime-v3 episode already exists; rerun with --resume after reviewing pending state",
                ));
            }
            let state = store
                .resume_or_start_episode(&lineage, &fingerprint)
                .map_err(|error| format!("cannot admit runtime-v3 episode: {error}"))?;
            let stored_episode = store
                .load_episode(&lineage.episode_id)
                .map_err(|error| format!("cannot load runtime-v3 execution episode: {error}"))?;
            (state, stored_episode)
        };
        if stored_episode.lineage != lineage {
            return Err(String::from(
                "runtime-v3 stored episode lineage does not match the approved runtime",
            ));
        }
        if stored_episode.fingerprint != fingerprint {
            return Err(String::from(
                "runtime-v3 stored episode fingerprint does not match the approved runtime",
            ));
        }
        if let ResumeState::Ready { checkpoint, .. } = &state
            && checkpoint.as_deref() != stored_episode.last_checkpoint.as_ref()
        {
            return Err(String::from(
                "runtime-v3 stored resume boundary does not match the latest checkpoint",
            ));
        }
        match &state {
            ResumeState::Completed(_) => {}
            ResumeState::ReconstructionRequired { reason }
            | ResumeState::InterruptedUnknown { reason } => {
                return Err(format!(
                    "runtime-v3 episode requires a separately approved reconstruction: {reason}"
                ));
            }
            ResumeState::New => {
                return Err(String::from(
                    "runtime-v3 episode admission returned an unexpected new state",
                ));
            }
            ResumeState::Ready { .. } => {}
        }
        let next_checkpoint =
            stored_episode
                .last_checkpoint
                .as_ref()
                .map_or(Ok(0_u64), |checkpoint| {
                    checkpoint
                        .sequence
                        .checked_add(1)
                        .ok_or_else(|| String::from("runtime-v3 checkpoint sequence exhausted"))
                })?;
        let resume_boundary = match &state {
            ResumeState::Ready { checkpoint, .. } => checkpoint.as_deref().cloned(),
            _ => None,
        };
        let handle = Self {
            store: share_store(store),
            lineage,
            fingerprint,
            model_revision: settings.exo.revision.clone(),
            config_digest: config_digest_with_binding(config, settings, &workflow_binding)?,
            next_checkpoint: Rc::new(RefCell::new(next_checkpoint)),
            resume_boundary: Rc::new(RefCell::new(resume_boundary)),
            workflow_binding,
            owns_store: true,
            worker_handoff: None,
        };
        Ok((handle, state))
    }

    pub(super) fn pending_operations(&self) -> Result<Vec<sts2_harness::StoredOperation>, String> {
        super::worker_store::try_lock_recovery(&self.store)?
            .pending_operations(&self.lineage.episode_id)
            .map_err(|error| format!("cannot read pending runtime-v3 operations: {error}"))
    }

    pub(super) fn close(&self) -> Result<(), String> {
        if !self.owns_store {
            return Ok(());
        }
        try_lock_close(&self.store)?
            .close()
            .map_err(|error| format!("cannot close runtime-v3 execution store: {error}"))
    }

    #[cfg(test)]
    pub(super) fn from_store_for_test(
        store: ExecutionStore,
        lineage: ExecutionLineage,
        fingerprint: ExecutionFingerprint,
    ) -> Result<Self, String> {
        Self::from_store_for_test_with_binding(
            store,
            lineage,
            fingerprint,
            WorkflowBinding::default_full_episode(),
        )
    }

    #[cfg(test)]
    pub(super) fn from_store_for_test_with_binding(
        store: ExecutionStore,
        lineage: ExecutionLineage,
        fingerprint: ExecutionFingerprint,
        workflow_binding: WorkflowBinding,
    ) -> Result<Self, String> {
        let stored = snapshot(&store, &lineage, &fingerprint)?;
        let next_checkpoint =
            stored
                .episode
                .last_checkpoint
                .as_ref()
                .map_or(Ok(0_u64), |checkpoint| {
                    checkpoint
                        .sequence
                        .checked_add(1)
                        .ok_or_else(|| String::from("test checkpoint sequence exhausted"))
                })?;
        let config_digest = fingerprint.config_digest.clone();
        Ok(Self {
            store: share_store(store),
            lineage,
            fingerprint,
            model_revision: String::from("synthetic-provider"),
            config_digest,
            next_checkpoint: Rc::new(RefCell::new(next_checkpoint)),
            resume_boundary: Rc::new(RefCell::new(None)),
            workflow_binding,
            owns_store: true,
            worker_handoff: None,
        })
    }
}

#[cfg(test)]
pub(super) fn validate_worker_config_path_for_test(config: &RuntimeConfig) -> Result<(), String> {
    admitted::validate_worker_config_path_for_test(config)
}

#[cfg(test)]
pub(super) fn config_digest_for_test(
    config: &RuntimeConfig,
    settings: &RuntimeV3Settings,
) -> Result<String, String> {
    admitted::config_digest_for_test(config, settings)
}

#[derive(Clone)]
pub(super) struct ProviderReservationToken {
    reservation_id: String,
}
