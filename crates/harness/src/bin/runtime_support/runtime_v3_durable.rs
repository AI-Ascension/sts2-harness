// SPDX-License-Identifier: MIT

//! Durable runtime-v3 state.  The gameplay port remains the owner of the live MCP transport;
//! this module owns the durable admission and evidence boundary around it.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sts2_harness::{
    Checkpoint, CompletionRecord, CompletionStatus, DecisionReference, ExecutionFingerprint,
    ExecutionLineage, ExecutionStore, ExecutionStoreConfig, OperationIntent, OperationState,
    ProviderFailureClass, ProviderReservation, RECOVERY_SCHEMA_DIGEST, ResumeState,
};
use sts2_harness::{
    Decision, DecisionInput, EpisodeLegalAction, EpisodeObservation, EpisodeRunReport,
};

use super::super::config::RuntimeConfig;
use super::super::runtime_v3_settings::RuntimeV3Settings;

const DEFAULT_STORE_PATH: &str = "harness-execution.sqlite3";
const DEFAULT_SEED: &str = "seed:unavailable";
const DEFAULT_BUILD: &str = "build:unavailable";
const DEFAULT_STATE: &str = "state:unavailable";
const PROVIDER_RESERVATION_UNITS: u64 = 1;

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
        let fingerprint = fingerprint(config, settings)?;
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
            ResumeState::Completed(_) => {
                return Err(String::from(
                    "durable runtime-v3 episode is already complete; refusing to rerun it",
                ));
            }
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
        let handle = Self {
            store: Rc::new(RefCell::new(store)),
            lineage,
            fingerprint,
            model_revision: settings.exo.revision.clone(),
            config_digest: config_digest(config, settings)?,
            next_checkpoint: Rc::new(RefCell::new(next_checkpoint)),
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

    pub(super) fn operation_intent(
        &self,
        operation_id: &str,
        state_id: &str,
        generation: u64,
        action: &EpisodeLegalAction,
        payload: &Value,
        input: &Value,
    ) -> Result<String, String> {
        let payload_digest = sha256_json(payload)?;
        let input_digest = sha256_json(input)?;
        let intent = OperationIntent::new(
            self.lineage.clone(),
            operation_id,
            state_id,
            generation,
            action.action_id(),
            payload_digest.clone(),
            input_digest,
        )
        .map_err(|error| format!("runtime-v3 operation intent is invalid: {error}"))?;
        self.store
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
            .record_operation_intent(&intent)
            .map_err(|error| format!("cannot persist runtime-v3 operation intent: {error}"))?;
        Ok(payload_digest)
    }

    pub(super) fn operation_dispatched(
        &self,
        operation_id: &str,
        payload_digest: &str,
    ) -> Result<(), String> {
        self.store
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
            .mark_operation_dispatched(operation_id, payload_digest)
            .map(|_| ())
            .map_err(|error| format!("cannot persist runtime-v3 dispatch intent: {error}"))
    }

    pub(super) fn operation_result(
        &self,
        operation_id: &str,
        payload_digest: &str,
        status: OperationState,
        response: Option<&Value>,
    ) -> Result<(), String> {
        let evidence = response
            .map(|value| response_evidence(operation_id, value))
            .transpose()?;
        self.store
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
            .record_operation_result(
                operation_id,
                payload_digest,
                status,
                evidence.as_ref().map(|(reference, _)| reference.as_str()),
                evidence.as_ref().map(|(_, digest)| digest.as_str()),
            )
            .map(|_| ())
            .map_err(|error| format!("cannot persist runtime-v3 operation result: {error}"))
    }

    pub(super) fn operation_state(&self, operation_id: &str) -> Result<OperationState, String> {
        self.store
            .try_borrow()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
            .operation(operation_id)
            .map(|operation| operation.state)
            .map_err(|error| format!("cannot read runtime-v3 operation state: {error}"))
    }

    pub(super) fn operation_payload_digest(&self, operation_id: &str) -> Result<String, String> {
        self.store
            .try_borrow()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
            .operation(operation_id)
            .map(|operation| operation.intent.payload_digest)
            .map_err(|error| format!("cannot read runtime-v3 operation digest: {error}"))
    }

    /// Applies a result from the authoritative recovery endpoint.  A recovery result resolves
    /// the original operation in one durable transition; it never creates a replacement ID.
    pub(super) fn reconcile_response(
        &self,
        operation_id: &str,
        resolved_state: OperationState,
        response: &Value,
    ) -> Result<(), String> {
        let operation = self
            .store
            .try_borrow()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
            .operation(operation_id)
            .map_err(|error| {
                format!("cannot read runtime-v3 operation for reconciliation: {error}")
            })?;
        let (result_ref, result_digest) = response_evidence(operation_id, response)?;
        self.store
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
            .reconcile_operation(
                operation_id,
                &operation.intent.payload_digest,
                resolved_state,
                &result_ref,
                &result_digest,
            )
            .map(|_| ())
            .map_err(|error| format!("cannot reconcile runtime-v3 operation: {error}"))
    }

    pub(super) fn decision_admission(
        &self,
        input: &DecisionInput,
    ) -> Result<Option<ProviderReservationToken>, String> {
        self.store
            .try_borrow()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
            .resume_for_decision(&self.lineage.episode_id, &self.fingerprint)
            .map_err(|error| format!("runtime-v3 decision admission is blocked: {error}"))?;
        let input_fingerprint = decision_input_digest(input)?;
        let execution_id = input.execution_id.to_string();
        let reference = DecisionReference::new(
            self.lineage.clone(),
            execution_id.clone(),
            input_fingerprint,
            self.model_revision.clone(),
            self.config_digest.clone(),
        )
        .map_err(|error| format!("runtime-v3 decision reference is invalid: {error}"))?;
        let stored = self
            .store
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
            .record_decision(&reference)
            .map_err(|error| format!("cannot persist runtime-v3 decision reference: {error}"))?;
        if stored.completed {
            return Ok(None);
        }
        if stored.unknown {
            return Err(String::from(
                "runtime-v3 provider decision is unknown and cannot be reused",
            ));
        }
        let reservation_id = format!("provider-reservation-{execution_id}");
        let provider_execution_id = format!("provider-execution-{execution_id}");
        let reservation = ProviderReservation::new(
            self.lineage.clone(),
            reservation_id.clone(),
            execution_id,
            provider_execution_id,
            PROVIDER_RESERVATION_UNITS,
        )
        .map_err(|error| format!("runtime-v3 provider reservation is invalid: {error}"))?;
        self.store
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
            .reserve_provider(&reservation)
            .map_err(|error| format!("cannot reserve runtime-v3 provider usage: {error}"))?;
        Ok(Some(ProviderReservationToken { reservation_id }))
    }

    pub(super) fn complete_decision(
        &self,
        token: &ProviderReservationToken,
        decision: &Decision,
    ) -> Result<(), String> {
        let result_digest = decision_digest(decision);
        let result_ref = format!("decision-result-{}", token.reservation_id);
        self.store
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
            .complete_provider(
                &token.reservation_id,
                &result_ref,
                &result_digest,
                PROVIDER_RESERVATION_UNITS,
            )
            .map(|_| ())
            .map_err(|error| format!("cannot persist runtime-v3 provider completion: {error}"))
    }

    pub(super) fn fail_decision(
        &self,
        token: &ProviderReservationToken,
        failure: ProviderFailureClass,
    ) -> Result<(), String> {
        self.store
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
            .fail_provider(&token.reservation_id, failure, None)
            .map(|_| ())
            .map_err(|error| format!("cannot persist runtime-v3 provider failure: {error}"))
    }

    pub(super) fn unknown_decision(
        &self,
        token: &ProviderReservationToken,
        failure: ProviderFailureClass,
    ) -> Result<(), String> {
        self.store
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
            .mark_provider_unknown(&token.reservation_id, failure, None)
            .map(|_| ())
            .map_err(|error| format!("cannot persist unknown runtime-v3 provider result: {error}"))
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
        let checkpoint_sequence = self
            .next_checkpoint
            .try_borrow()
            .map_err(|_| String::from("runtime-v3 checkpoint sequence is already borrowed"))?
            .checked_sub(1)
            .ok_or_else(|| String::from("runtime-v3 terminal observation was not checkpointed"))?;
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
}

#[derive(Clone)]
pub(super) struct ProviderReservationToken {
    reservation_id: String,
}

fn fingerprint(
    config: &RuntimeConfig,
    settings: &RuntimeV3Settings,
) -> Result<ExecutionFingerprint, String> {
    let seed = optional_env("STS2_SEED")?
        .or(optional_env("STS2_VISIBLE_SEED")?)
        .map_or_else(
            || digest_text(DEFAULT_SEED),
            |value| reference_or_digest(&value),
        );
    let build = optional_env("STS2_BUILD_DIGEST")?.map_or_else(
        || digest_text(DEFAULT_BUILD),
        |value| reference_or_digest(&value),
    );
    let state = optional_env("STS2_STATE_DIGEST")?.map_or_else(
        || digest_text(DEFAULT_STATE),
        |value| reference_or_digest(&value),
    );
    let config_digest = config_digest(config, settings)?;
    let provider_digest = reference_or_digest(&settings.exo.revision);
    ExecutionFingerprint::new(seed, build, state, config_digest, provider_digest)
        .map_err(|error| format!("runtime-v3 execution fingerprint is invalid: {error}"))
}

fn config_digest(config: &RuntimeConfig, settings: &RuntimeV3Settings) -> Result<String, String> {
    let value = json!({
        "runtime_profile": config.runtime_profile,
        "gateway_address": config.gateway_address,
        "mcp_binary": config.mcp_binary,
        "instance_id": config.instance_id,
        "caller_id": config.caller_id,
        "session_id": config.session_id,
        "lease_id": config.lease_id,
        "lease_epoch": config.lease_epoch,
        "mcp_session_id": config.mcp_session_id,
        "run_id": config.run_id,
        "episode_id": config.episode_id,
        "trajectory_id": config.trajectory_id,
        "trace_id": config.trace_id,
        "artifact_id": config.artifact_id,
        "settlement_timeout_seconds": config.settlement_timeout_seconds,
        "exo_revision": settings.exo.revision,
        "exo_max_request_bytes": settings.exo.max_request_bytes,
        "exo_max_response_bytes": settings.exo.max_response_bytes,
        "exo_timeout_millis": settings.exo.timeout_millis,
        "exo_forward_visible_seed": settings.exo.forward_visible_seed,
        "exo_bridge": {
            "executable": settings.process.executable(),
            "arguments": settings.process.arguments(),
            "working_directory": settings.process.working_directory(),
            "inherited_environment": settings.process.inherited_environment(),
        },
        "runner": {
            "max_steps": settings.runner.max_steps(),
            "objective": settings.runner.objective(),
            "hard_constraints": settings.runner.hard_constraints(),
        },
    });
    sha256_json(&value)
}

fn decision_input_digest(input: &DecisionInput) -> Result<String, String> {
    let legal_actions: Vec<_> = input
        .legal_actions
        .actions()
        .iter()
        .map(|action| {
            json!({
                "action_id": action.action_id(),
                "kind": super::super::runtime_v3_wire::action_kind_name(action.kind()),
            })
        })
        .collect();
    sha256_json(&json!({
        "state_id": input.observation.state_id(),
        "generation": input.observation.generation(),
        "fair_play": input.observation.fair_play().as_value(),
        "legal_actions": legal_actions,
        "objective": input.objective,
        "hard_constraints": input.hard_constraints,
    }))
}

fn decision_digest(decision: &Decision) -> String {
    let summary = match decision {
        Decision::Plan { action_ids, .. } => json!({"decision":"plan", "action_ids":action_ids}),
        Decision::Action {
            action_id,
            confidence,
            ..
        } => {
            json!({"decision":"action", "action_id":action_id, "confidence":confidence})
        }
        Decision::Wait { .. } => json!({"decision":"wait"}),
        Decision::Reobserve { .. } => json!({"decision":"reobserve"}),
        Decision::Recovery {
            kind, operation_id, ..
        } => {
            json!({"decision":"recovery", "kind":kind, "operation_id":operation_id})
        }
    };
    // This function only receives bounded, already validated semantic fields, so hashing cannot
    // fail for serialization reasons.  Keep the fallback deterministic if the JSON API changes.
    serde_json::to_vec(&summary)
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
        .unwrap_or_else(|_| digest_text("decision:serialization-failed"))
}

fn response_evidence(operation_id: &str, response: &Value) -> Result<(String, String), String> {
    Ok((
        format!("mcp-response-{operation_id}"),
        sha256_json(response)?,
    ))
}

fn sha256_json(value: &Value) -> Result<String, String> {
    serde_json::to_vec(value)
        .map(|bytes| format!("{:x}", Sha256::digest(bytes)))
        .map_err(|error| format!("cannot hash runtime-v3 evidence: {error}"))
}

fn digest_text(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn reference_or_digest(value: &str) -> String {
    if !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control) {
        value.to_owned()
    } else {
        digest_text(value)
    }
}

fn optional_env(name: &str) -> Result<Option<String>, String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Ok(Some(value)),
        Ok(_) => Err(format!("{name} must not be empty")),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} is not valid UTF-8")),
    }
}
