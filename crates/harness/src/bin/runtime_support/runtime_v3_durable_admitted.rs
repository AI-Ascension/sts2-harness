// SPDX-License-Identifier: MIT

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use sts2_harness::{ExecutionFingerprint, ExecutionLineage, ResumeState};

use super::super::super::config::RuntimeConfig;
use super::super::super::runtime_v3_settings::RuntimeV3Settings;
use super::super::worker_store::{SharedExecutionStore, try_lock};
use super::DurableHandle;
use super::support::config_digest;

impl DurableHandle {
    /// Attaches the runtime to a handoff that the caller has already admitted and marked
    /// running. This path never opens a connection, starts an episode, or grants execution
    /// authority; it only verifies the existing handoff and resumes its exact durable identity.
    // This boundary is consumed by the separately owned worker entry path.
    #[allow(dead_code)]
    pub(crate) fn from_admitted_shared_store(
        store: SharedExecutionStore,
        handoff: &sts2_harness::StoredWorkerHandoff,
        config: &RuntimeConfig,
        settings: &RuntimeV3Settings,
        lineage: ExecutionLineage,
        fingerprint: ExecutionFingerprint,
    ) -> Result<(Self, ResumeState), String> {
        validate_worker_config(config)?;
        handoff
            .tuple
            .validate()
            .map_err(|error| format!("runtime-v3 worker handoff is invalid: {error}"))?;
        if handoff.state != sts2_harness::WorkerHandoffState::Running {
            return Err(String::from(
                "runtime-v3 worker handoff is not in the running state",
            ));
        }
        if worker_lineage(&handoff.tuple)? != lineage
            || config.run_id != lineage.run_id
            || config.episode_id != lineage.episode_id
            || config.trajectory_id != lineage.trajectory_id
        {
            return Err(String::from(
                "runtime-v3 worker handoff lineage does not match the approved runtime",
            ));
        }
        let expected_config_digest = config_digest(config, settings)?;
        if expected_config_digest != fingerprint.config_digest {
            return Err(String::from(
                "runtime-v3 approved fingerprint does not match the runtime configuration",
            ));
        }
        let (current, state) = {
            let store_guard = try_lock(&store)?;
            let current = store_guard
                .worker_handoff(&handoff.tuple.handoff_id)
                .map_err(|error| format!("cannot inspect runtime-v3 worker handoff: {error}"))?
                .ok_or_else(|| String::from("runtime-v3 worker handoff is missing"))?;
            if current != *handoff || current.state != sts2_harness::WorkerHandoffState::Running {
                return Err(String::from(
                    "runtime-v3 worker handoff changed before runtime attachment",
                ));
            }
            let state = store_guard
                .resume_episode(&lineage.episode_id, &fingerprint)
                .map_err(|error| format!("cannot inspect runtime-v3 execution state: {error}"))?;
            (current, state)
        };
        if worker_lineage(&current.tuple)? != lineage {
            return Err(String::from(
                "runtime-v3 stored worker handoff lineage does not match the approved runtime",
            ));
        }
        let resume_boundary = match &state {
            ResumeState::Ready { checkpoint, .. } => checkpoint.as_deref().cloned(),
            ResumeState::Completed(_) => {
                return Err(String::from(
                    "runtime-v3 worker handoff targets an already completed episode",
                ));
            }
            ResumeState::New => {
                return Err(String::from(
                    "runtime-v3 worker handoff targets a missing episode",
                ));
            }
            ResumeState::ReconstructionRequired { reason }
            | ResumeState::InterruptedUnknown { reason } => {
                return Err(format!(
                    "runtime-v3 worker episode cannot be attached: {reason}"
                ));
            }
        };
        let next_checkpoint = {
            let store_guard = try_lock(&store)?;
            store_guard
                .last_checkpoint(&lineage.episode_id)
                .map_err(|error| format!("cannot read runtime-v3 checkpoint: {error}"))?
                .map_or(Ok(0_u64), |checkpoint| {
                    checkpoint
                        .sequence
                        .checked_add(1)
                        .ok_or_else(|| String::from("runtime-v3 checkpoint sequence exhausted"))
                })?
        };
        let handle = Self {
            store,
            lineage,
            fingerprint,
            model_revision: settings.exo.revision.clone(),
            config_digest: expected_config_digest,
            next_checkpoint: Rc::new(RefCell::new(next_checkpoint)),
            resume_boundary: Rc::new(RefCell::new(resume_boundary)),
            owns_store: false,
        };
        Ok((handle, state))
    }

    #[cfg(test)]
    pub(crate) fn from_shared_store_for_test(
        store: SharedExecutionStore,
        lineage: ExecutionLineage,
        fingerprint: ExecutionFingerprint,
    ) -> Result<Self, String> {
        let next_checkpoint = try_lock(&store)?
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
            store,
            lineage,
            fingerprint,
            model_revision: String::from("synthetic-provider"),
            config_digest,
            next_checkpoint: Rc::new(RefCell::new(next_checkpoint)),
            resume_boundary: Rc::new(RefCell::new(None)),
            owns_store: false,
        })
    }
}

fn worker_lineage(tuple: &sts2_harness::WorkerTuple) -> Result<ExecutionLineage, String> {
    ExecutionLineage::new(
        tuple.run_id.clone(),
        tuple.episode_id.clone(),
        tuple.attempt_id.clone(),
        tuple.trajectory_id.clone(),
    )
    .map_err(|error| format!("runtime-v3 worker lineage is invalid: {error}"))
}

fn validate_worker_config(config: &RuntimeConfig) -> Result<(), String> {
    if !Path::new(&config.mcp_binary).is_absolute() {
        return Err(String::from(
            "runtime-v3 worker attachment requires an absolute approved MCP executable path",
        ));
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn validate_worker_config_path_for_test(config: &RuntimeConfig) -> Result<(), String> {
    validate_worker_config(config)
}

#[cfg(test)]
pub(super) fn config_digest_for_test(
    config: &RuntimeConfig,
    settings: &RuntimeV3Settings,
) -> Result<String, String> {
    config_digest(config, settings)
}
