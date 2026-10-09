// SPDX-License-Identifier: MIT

use serde_json::Value;
use sts2_harness::recipe::contract_v2::{RecipeDefinitionV2, RecipeOperationV2, admit_recipe_v2};
use sts2_harness::{
    DecisionInput, EpisodeLegalActionSet, ModelExecutionId, RecipeInvocationBinding,
    RecipeInvocationContext, RecipeInvocationReceipt, RecipeInvocationStatus,
};

use super::DurableHandle;
use super::support::sha256_bytes;

#[path = "runtime_v3_map_collection_error.rs"]
mod map_collection_error;
#[path = "runtime_v3_map_result_identity.rs"]
mod map_result_identity;

use map_collection_error::MapCollectionError;
use map_result_identity::map_result_digest;

const MAP_RECIPE_ID: &str = "runtime.map-context";
const MAP_PROFILE: &str = "runtime-map-v1";
const RPC_CORRELATION_ID: &str = "3";

#[cfg(test)]
fn collect_map_snapshot<F>(
    durable: &DurableHandle,
    state_id: &str,
    generation: u64,
    execution_id: ModelExecutionId,
    actions: &EpisodeLegalActionSet,
    read: F,
) -> Result<Value, String>
where
    F: FnOnce() -> Result<Value, String>,
{
    collect_map_snapshot_classified(durable, state_id, generation, execution_id, actions, read)
        .map_err(MapCollectionError::into_message)
}

pub(in super::super) fn collect_map_snapshot_for_port<F>(
    durable: &DurableHandle,
    state_id: &str,
    generation: u64,
    execution_id: ModelExecutionId,
    actions: &EpisodeLegalActionSet,
    read: F,
) -> Result<Value, sts2_harness::PortError>
where
    F: FnOnce() -> Result<Value, String>,
{
    collect_map_snapshot_classified(durable, state_id, generation, execution_id, actions, read)
        .map_err(MapCollectionError::into_port_error)
}

fn collect_map_snapshot_classified<F>(
    durable: &DurableHandle,
    state_id: &str,
    generation: u64,
    execution_id: ModelExecutionId,
    actions: &EpisodeLegalActionSet,
    read: F,
) -> Result<Value, MapCollectionError>
where
    F: FnOnce() -> Result<Value, String>,
{
    actions
        .assert_matches(state_id, generation)
        .map_err(|_| MapCollectionError::failed("runtime map action catalog is stale"))?;
    let binding = map_invocation_binding(durable, execution_id, state_id, generation)
        .map_err(MapCollectionError::failed)?;
    durable
        .begin_map_invocation(binding.clone())
        .map_err(MapCollectionError::failed)?;
    let value = read().map_err(MapCollectionError::failed)?;
    let result_digest = map_result_identity::typed_result_digest(&value, &binding, actions)?;
    durable
        .record_map_invocation_response(&binding, &result_digest)
        .map_err(MapCollectionError::failed)?;
    Ok(value)
}

fn map_invocation_binding(
    durable: &DurableHandle,
    execution_id: ModelExecutionId,
    state_id: &str,
    generation: u64,
) -> Result<RecipeInvocationBinding, String> {
    let definition = RecipeDefinitionV2::new(MAP_RECIPE_ID, 1, RecipeOperationV2::MapSnapshot {});
    let admitted = admit_recipe_v2(definition)
        .map_err(|_| String::from("runtime map recipe admission failed"))?;
    RecipeInvocationBinding::new(
        durable.lifecycle_lineage(),
        execution_id,
        &admitted,
        RecipeInvocationContext {
            runtime_config_digest: durable.lifecycle_config_digest(),
            instance_id: durable.map_instance_id.clone(),
            gateway_session_id: durable.map_gateway_session_id.clone(),
            mcp_session_id: durable.map_mcp_session_id.clone(),
            lease_id: durable.map_lease_id.clone(),
            lease_epoch: durable.map_lease_epoch,
            state_id: state_id.to_owned(),
            generation,
        },
    )
    .map_err(|_| String::from("runtime map recipe binding is invalid"))
}

impl DurableHandle {
    pub(super) fn begin_map_invocation(
        &self,
        binding: RecipeInvocationBinding,
    ) -> Result<(), String> {
        let mut pending = self
            .pending_recipe_invocation
            .try_borrow_mut()
            .map_err(|_| String::from("runtime map receipt is already borrowed"))?;
        if pending.is_some() {
            return Err(String::from("another runtime map invocation is pending"));
        }
        let created = self
            .store
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
            .record_recipe_invocation_intent(&binding)
            .map_err(|error| format!("cannot persist runtime map invocation intent: {error}"))?;
        if !created {
            return Err(String::from(
                "runtime map invocation already exists; refusing to repeat its read",
            ));
        }
        *pending = Some(binding);
        Ok(())
    }

    pub(super) fn record_map_invocation_response(
        &self,
        binding: &RecipeInvocationBinding,
        result_digest: &str,
    ) -> Result<(), String> {
        if self.pending_map_invocation()?.as_ref() != Some(binding) {
            return Err(String::from("runtime map response has no matching intent"));
        }
        self.store
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
            .record_recipe_invocation_response(binding, result_digest)
            .map_err(|error| format!("cannot persist runtime map response digest: {error}"))
    }

    pub(super) fn pending_map_invocation(&self) -> Result<Option<RecipeInvocationBinding>, String> {
        self.pending_recipe_invocation
            .try_borrow()
            .map(|pending| (*pending).clone())
            .map_err(|_| String::from("runtime map receipt is already borrowed"))
    }

    pub(super) fn map_invocation_receipt(
        &self,
        binding: &RecipeInvocationBinding,
    ) -> Result<Option<RecipeInvocationReceipt>, String> {
        self.store
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
            .recipe_invocation_receipt(binding)
            .map_err(|error| format!("cannot inspect runtime map invocation receipt: {error}"))
    }

    pub(super) fn finalize_map_invocation(
        &self,
        binding: &RecipeInvocationBinding,
        owner_snapshot_digest: &str,
        decision_input_digest: &str,
    ) -> Result<(), String> {
        let mut pending = self
            .pending_recipe_invocation
            .try_borrow_mut()
            .map_err(|_| String::from("runtime map receipt is already borrowed"))?;
        if pending.as_ref() != Some(binding) {
            return Err(String::from(
                "runtime map response was not read in this process",
            ));
        }
        self.store
            .try_borrow_mut()
            .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
            .finalize_recipe_invocation(binding, owner_snapshot_digest, decision_input_digest)
            .map_err(|error| format!("cannot finalize runtime map context receipt: {error}"))?;
        *pending = None;
        Ok(())
    }

    pub(super) fn clear_revalidated_map_invocation(
        &self,
        binding: &RecipeInvocationBinding,
    ) -> Result<(), String> {
        let mut pending = self
            .pending_recipe_invocation
            .try_borrow_mut()
            .map_err(|_| String::from("runtime map receipt is already borrowed"))?;
        if pending.as_ref().is_some_and(|value| value != binding) {
            return Err(String::from("runtime map receipt binding changed"));
        }
        *pending = None;
        Ok(())
    }
}

pub(super) fn finalize_decision_context(
    durable: &DurableHandle,
    input: &DecisionInput,
    input_fingerprint: &str,
) -> Result<String, String> {
    if !is_sha256(input_fingerprint) {
        return Err(String::from("runtime decision fingerprint is invalid"));
    }
    let Some(owner_snapshot_digest) = input.accepted_map_snapshot_digest() else {
        if durable.pending_map_invocation()?.is_some()
            || durable
                .store
                .try_borrow_mut()
                .map_err(|_| String::from("runtime-v3 execution store is already borrowed"))?
                .recipe_invocation_exists_for_execution(&durable.lineage, input.execution_id)
                .map_err(|error| format!("runtime map receipt query failed: {error}"))?
        {
            return Err(String::from(
                "runtime map invocation has no owner-validated decision context",
            ));
        }
        return Ok(input_fingerprint.to_owned());
    };
    let binding = map_invocation_binding(
        durable,
        input.execution_id,
        input.observation.state_id(),
        input.observation.generation(),
    )?;
    let pending = durable.pending_map_invocation()?;
    let receipt = durable.map_invocation_receipt(&binding)?;
    if pending.as_ref().is_some_and(|value| value != &binding) {
        return Err(String::from("runtime map receipt binding changed"));
    }
    let receipt = receipt.ok_or_else(|| {
        String::from("owner-validated map context has no matching durable response")
    })?;
    let effective_fingerprint =
        map_decision_input_digest(input_fingerprint, owner_snapshot_digest)?;
    match receipt.status() {
        RecipeInvocationStatus::IntentRecorded => Err(String::from(
            "runtime map response is not durably validated",
        )),
        RecipeInvocationStatus::ResponseValidated => {
            if pending.as_ref() != Some(&binding) {
                return Err(String::from(
                    "unfinished runtime map receipt cannot be resumed after restart",
                ));
            }
            let expected_result_digest = map_result_digest(owner_snapshot_digest);
            if receipt.typed_result_digest() != Some(expected_result_digest.as_str()) {
                return Err(String::from(
                    "runtime map response does not match the owner-accepted snapshot",
                ));
            }
            durable.finalize_map_invocation(
                &binding,
                owner_snapshot_digest,
                &effective_fingerprint,
            )?;
            Ok(effective_fingerprint)
        }
        RecipeInvocationStatus::ContextValidated => {
            if receipt.owner_snapshot_digest() != Some(owner_snapshot_digest)
                || receipt.decision_input_digest() != Some(effective_fingerprint.as_str())
            {
                return Err(String::from(
                    "runtime map context differs from its finalized decision receipt",
                ));
            }
            durable.clear_revalidated_map_invocation(&binding)?;
            Ok(effective_fingerprint)
        }
    }
}

fn map_decision_input_digest(
    input_fingerprint: &str,
    owner_snapshot_digest: &str,
) -> Result<String, String> {
    if !is_sha256(input_fingerprint) || !is_sha256(owner_snapshot_digest) {
        return Err(String::from("runtime map decision digest input is invalid"));
    }
    let mut material = Vec::with_capacity(256);
    material.extend_from_slice(b"sts2-harness:recipe-map-decision-input:v1\0");
    for field in [
        input_fingerprint,
        MAP_PROFILE,
        sts2_harness::RUNTIME_MAP_SCHEMA_DIGEST,
        owner_snapshot_digest,
    ] {
        material.extend_from_slice(field.as_bytes());
        material.push(0);
    }
    Ok(sha256_bytes(&material))
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
#[path = "runtime_v3_durable_recipe_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "runtime_v3_map_collection_error_tests.rs"]
mod map_collection_error_tests;
