// SPDX-License-Identifier: MIT

//! Durable runtime-v3 state and its admission/evidence boundary around the gameplay MCP transport.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use serde_json::Value;
use sts2_harness::{
    Checkpoint, CompletionRecord, CompletionStatus, EpisodeObservation, EpisodeRunReport,
    ExecutionFingerprint, ExecutionLineage, ExecutionStore, ExecutionStoreConfig,
    RECOVERY_SCHEMA_DIGEST, ResumeState,
};

use super::super::config::RuntimeConfig;
use super::super::runtime_v3_settings::RuntimeV3Settings;

const DEFAULT_STORE_PATH: &str = "harness-execution.sqlite3";
const PROVIDER_RESERVATION_UNITS: u64 = 1;

#[path = "runtime_v3_durable_operations.rs"]
mod operations;
#[path = "runtime_v3_durable_support.rs"]
mod support;

use support::{config_digest, fingerprint, optional_env, sha256_json};

/// A cloneable handle deliberately backed by one owner-local SQLite connection.
///
/// The runtime port, provider recorder, and test seams all use this handle, but each database
/// borrow is kept short.  No store connection is shared across processes or threads.
#[derive(Clone)]
pub(super) struct DurableHandle {
    store: Rc<RefCell<ExecutionStore>>,
    lineage: ExecutionLineage,
    fingerprint: ExecutionFingerprint,
    model_revision: String,
    config_digest: String,
    next_checkpoint: Rc<RefCell<u64>>,
    resume_boundary: Rc<RefCell<Option<Checkpoint>>>,
}

impl DurableHandle {
    pub(super) fn open(
        config: &RuntimeConfig,
        settings: &RuntimeV3Settings,
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
        let fingerprint = fingerprint(config, settings, resume_requested)?;
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
        let next_checkpoint = store
            .last_checkpoint(&lineage.episode_id)
            .map_err(|error| format!("cannot read runtime-v3 checkpoint: {error}"))?
            .map_or(Ok(0_u64), |checkpoint| {
                checkpoint
                    .sequence
                    .checked_add(1)
                    .ok_or_else(|| String::from("runtime-v3 checkpoint sequence exhausted"))
            })?;
        let resume_boundary = match &state {
            ResumeState::Ready { checkpoint, .. } => checkpoint.clone(),
            _ => None,
        };
        let handle = Self {
            store: Rc::new(RefCell::new(store)),
            lineage,
            fingerprint,
            model_revision: settings.exo.revision.clone(),
            config_digest: config_digest(config, settings)?,
            next_checkpoint: Rc::new(RefCell::new(next_checkpoint)),
            resume_boundary: Rc::new(RefCell::new(resume_boundary)),
        };
        Ok((handle, state))
    }

    pub(super) fn pending_operations(&self) -> Result<Vec<sts2_harness::StoredOperation>, String> {
        self.store
            .try_borrow()
            .map_err(|_| String::from("runtime-v3 execution store is already mutably borrowed"))?
            .pending_operations(&self.lineage.episode_id)
            .map_err(|error| format!("cannot read pending runtime-v3 operations: {error}"))
    }

    /// Requires the first fresh host observation after an explicit resume to equal the last
    /// durable public boundary. A mismatch cannot be turned into a new provider decision.
    pub(super) fn verify_resume_boundary(
        &self,
        observation: &EpisodeObservation,
    ) -> Result<(), String> {
        let observation_bytes = serde_json::to_vec(observation.fair_play().as_value())
            .map_err(|error| format!("cannot encode runtime-v3 resume observation: {error}"))?;
        let mut boundary = self
            .resume_boundary
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 resume boundary is already borrowed"))?;
        let Some(expected) = boundary.as_ref() else {
            return Ok(());
        };
        if expected.lineage != self.lineage
            || expected.fingerprint != self.fingerprint
            || expected.state_id != observation.state_id()
            || expected.generation != observation.generation()
            || expected.observation != observation_bytes
        {
            return Err(String::from(
                "runtime-v3 fresh observation does not match the verified resume boundary",
            ));
        }
        *boundary = None;
        Ok(())
    }

    /// Makes the latest checkpoint the next resume boundary after recovery has produced an
    /// authoritative observation. This is deliberately called only after all pending mutations
    /// have been reconciled.
    pub(super) fn refresh_resume_boundary(&self) -> Result<(), String> {
        let checkpoint = self
            .store
            .try_borrow()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
            .last_checkpoint(&self.lineage.episode_id)
            .map_err(|error| format!("cannot refresh runtime-v3 resume boundary: {error}"))?;
        *self
            .resume_boundary
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 resume boundary is already borrowed"))? =
            checkpoint;
        Ok(())
    }

    pub(super) fn checkpoint(
        &self,
        observation: &EpisodeObservation,
        payloads: &Value,
    ) -> Result<(), String> {
        let observation_bytes = serde_json::to_vec(observation.fair_play().as_value())
            .map_err(|error| format!("cannot encode runtime-v3 checkpoint: {error}"))?;
        let legal_actions_digest = sha256_json(payloads)?;
        let mut sequence = self
            .next_checkpoint
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 checkpoint sequence is already borrowed"))?;
        let checkpoint = Checkpoint::new(
            self.lineage.clone(),
            *sequence,
            observation.state_id(),
            observation.generation(),
            self.fingerprint.clone(),
            observation_bytes,
            legal_actions_digest,
        )
        .map_err(|error| format!("runtime-v3 checkpoint is invalid: {error}"))?;
        self.store
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
            .save_checkpoint(&checkpoint)
            .map_err(|error| format!("cannot persist runtime-v3 checkpoint: {error}"))?;
        *sequence = sequence
            .checked_add(1)
            .ok_or_else(|| String::from("runtime-v3 checkpoint sequence exhausted"))?;
        Ok(())
    }

    pub(super) fn complete_episode(&self, report: &EpisodeRunReport) -> Result<(), String> {
        self.complete_observation(report.final_observation())
    }

    pub(super) fn complete_observation(
        &self,
        observation: &EpisodeObservation,
    ) -> Result<(), String> {
        if !observation.stage().is_terminal() {
            return Err(String::from(
                "runtime-v3 completion requires a terminal observation",
            ));
        }
        let observation_bytes = serde_json::to_vec(observation.fair_play().as_value())
            .map_err(|error| format!("cannot encode runtime-v3 terminal observation: {error}"))?;
        let expected_next = self
            .next_checkpoint
            .try_borrow()
            .map_err(|_| String::from("runtime-v3 checkpoint sequence is already borrowed"))?;
        let checkpoint = self
            .store
            .try_borrow()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
            .last_checkpoint(&self.lineage.episode_id)
            .map_err(|error| format!("cannot read runtime-v3 terminal checkpoint: {error}"))?
            .ok_or_else(|| String::from("runtime-v3 terminal observation was not checkpointed"))?;
        if checkpoint.sequence.checked_add(1) != Some(*expected_next)
            || checkpoint.lineage != self.lineage
            || checkpoint.fingerprint != self.fingerprint
            || checkpoint.state_id != observation.state_id()
            || checkpoint.generation != observation.generation()
            || checkpoint.observation != observation_bytes
        {
            return Err(String::from(
                "runtime-v3 terminal observation does not match the latest checkpoint",
            ));
        }
        let checkpoint_sequence = checkpoint.sequence;
        let terminal_ref = format!(
            "terminal-{}-{}",
            super::super::runtime_v3_wire::stage_name(observation.stage()),
            observation.generation()
        );
        let result_digest = sha256_json(observation.fair_play().as_value())?;
        let completion = CompletionRecord::new(
            self.lineage.clone(),
            CompletionStatus::Completed,
            terminal_ref,
            checkpoint_sequence,
            result_digest,
        )
        .map_err(|error| format!("runtime-v3 completion is invalid: {error}"))?;
        self.store
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
            .record_completion(&completion)
            .map(|_| ())
            .map_err(|error| format!("cannot persist runtime-v3 completion: {error}"))
    }

    pub(super) fn mark_interrupted_unknown(&self, reason: &str) {
        if let Ok(mut store) = self.store.try_borrow_mut() {
            let _ = store.mark_interrupted_unknown(&self.lineage.episode_id, reason);
        }
    }

    pub(super) fn close(&self) -> Result<(), String> {
        self.store
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
            .close()
            .map_err(|error| format!("cannot close runtime-v3 execution store: {error}"))
    }

    #[cfg(test)]
    pub(super) fn from_store_for_test(
        store: ExecutionStore,
        lineage: ExecutionLineage,
        fingerprint: ExecutionFingerprint,
    ) -> Result<Self, String> {
        let next_checkpoint = store
            .last_checkpoint(&lineage.episode_id)
            .map_err(|error| format!("cannot read test checkpoint: {error}"))?
            .map_or(Ok(0_u64), |checkpoint| {
                checkpoint
                    .sequence
                    .checked_add(1)
                    .ok_or_else(|| String::from("test checkpoint sequence exhausted"))
            })?;
        let config_digest = fingerprint.config_digest.clone();
        Ok(Self {
            store: Rc::new(RefCell::new(store)),
            lineage,
            fingerprint,
            model_revision: String::from("synthetic-provider"),
            config_digest,
            next_checkpoint: Rc::new(RefCell::new(next_checkpoint)),
            resume_boundary: Rc::new(RefCell::new(None)),
        })
    }
}

#[derive(Clone)]
pub(super) struct ProviderReservationToken {
    reservation_id: String,
}
