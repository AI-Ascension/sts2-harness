// SPDX-License-Identifier: MIT

use rusqlite::{Connection, OptionalExtension, params};

use crate::identity::ModelExecutionId;

use super::super::schema;
use super::super::types::recipe_invocation::{
    MAP_RECEIPT_PROFILE, MAP_RECIPE_ID, MAP_RECIPE_OPERATION, MAP_RECIPE_REVISION,
    MAP_RPC_CORRELATION_ID, RecipeInvocationBinding, RecipeInvocationContext,
    RecipeInvocationStatus,
};
use super::super::types::{ExecutionLineage, ExecutionStoreError, valid_digest};
use super::StoredRecipeInvocation;

struct RecipeInvocationRow {
    runtime_config_digest: String,
    instance_id: String,
    gateway_session_id: String,
    mcp_session_id: String,
    lease_id: String,
    lease_epoch: i64,
    state_id: String,
    generation: i64,
    runtime_map_profile: String,
    schema_digest: String,
    rpc_correlation_id: String,
    request_digest: String,
    typed_result_digest: Option<String>,
    owner_snapshot_digest: Option<String>,
    decision_input_digest: Option<String>,
    state: String,
}

pub(super) fn invocation_exists_for_execution(
    connection: &Connection,
    lineage: &ExecutionLineage,
    model_execution_id: ModelExecutionId,
) -> Result<bool, ExecutionStoreError> {
    connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM recipe_map_invocations
                WHERE run_id = ?1 AND episode_id = ?2 AND attempt_id = ?3
                  AND trajectory_id = ?4 AND model_execution_id = ?5
                  AND recipe_id = ?6 AND recipe_revision = ?7 AND operation = ?8
            )",
            params![
                lineage.run_id,
                lineage.episode_id,
                lineage.attempt_id,
                lineage.trajectory_id,
                model_execution_id.to_string(),
                MAP_RECIPE_ID,
                MAP_RECIPE_REVISION,
                MAP_RECIPE_OPERATION,
            ],
            |row| row.get(0),
        )
        .map_err(schema::map_sqlite)
}

pub(super) fn insert_intent(
    transaction: &rusqlite::Transaction<'_>,
    binding: &RecipeInvocationBinding,
    request_digest: &str,
    now: i64,
) -> Result<(), ExecutionStoreError> {
    let context = &binding.context;
    transaction
        .execute(
            "INSERT INTO recipe_map_invocations(
                run_id, episode_id, attempt_id, trajectory_id, model_execution_id,
                recipe_id, recipe_revision, operation, runtime_config_digest,
                instance_id, gateway_session_id, mcp_session_id, lease_id, lease_epoch,
                state_id, generation, runtime_map_profile, schema_digest, rpc_correlation_id,
                request_digest, typed_result_digest, owner_snapshot_digest,
                decision_input_digest, state, created_at, updated_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                ?15, ?16, ?17, ?18, ?19, ?20, NULL, NULL, NULL, 'intent_recorded', ?21, ?21
             )",
            params![
                binding.lineage.run_id,
                binding.lineage.episode_id,
                binding.lineage.attempt_id,
                binding.lineage.trajectory_id,
                binding.model_execution_id,
                MAP_RECIPE_ID,
                MAP_RECIPE_REVISION,
                MAP_RECIPE_OPERATION,
                context.runtime_config_digest,
                context.instance_id,
                context.gateway_session_id,
                context.mcp_session_id,
                context.lease_id,
                i64::try_from(context.lease_epoch)
                    .map_err(|_| ExecutionStoreError::InvalidOperation)?,
                context.state_id,
                i64::try_from(context.generation)
                    .map_err(|_| ExecutionStoreError::InvalidOperation)?,
                MAP_RECEIPT_PROFILE,
                crate::RUNTIME_MAP_SCHEMA_DIGEST,
                MAP_RPC_CORRELATION_ID,
                request_digest,
                now,
            ],
        )
        .map_err(schema::map_sqlite)?;
    Ok(())
}

pub(super) fn load_invocation(
    connection: &Connection,
    binding: &RecipeInvocationBinding,
) -> Result<Option<StoredRecipeInvocation>, ExecutionStoreError> {
    let row = connection
        .query_row(
            "SELECT runtime_config_digest, instance_id, gateway_session_id, mcp_session_id,
                    lease_id, lease_epoch, state_id, generation, runtime_map_profile,
                    schema_digest, rpc_correlation_id, request_digest, typed_result_digest,
                    owner_snapshot_digest, decision_input_digest, state
             FROM recipe_map_invocations
             WHERE run_id = ?1 AND episode_id = ?2 AND attempt_id = ?3 AND trajectory_id = ?4
               AND model_execution_id = ?5 AND recipe_id = ?6 AND recipe_revision = ?7
               AND operation = ?8",
            params![
                binding.lineage.run_id,
                binding.lineage.episode_id,
                binding.lineage.attempt_id,
                binding.lineage.trajectory_id,
                binding.model_execution_id,
                MAP_RECIPE_ID,
                MAP_RECIPE_REVISION,
                MAP_RECIPE_OPERATION,
            ],
            |row| {
                Ok(RecipeInvocationRow {
                    runtime_config_digest: row.get::<_, String>(0)?,
                    instance_id: row.get::<_, String>(1)?,
                    gateway_session_id: row.get::<_, String>(2)?,
                    mcp_session_id: row.get::<_, String>(3)?,
                    lease_id: row.get::<_, String>(4)?,
                    lease_epoch: row.get::<_, i64>(5)?,
                    state_id: row.get::<_, String>(6)?,
                    generation: row.get::<_, i64>(7)?,
                    runtime_map_profile: row.get::<_, String>(8)?,
                    schema_digest: row.get::<_, String>(9)?,
                    rpc_correlation_id: row.get::<_, String>(10)?,
                    request_digest: row.get::<_, String>(11)?,
                    typed_result_digest: row.get::<_, Option<String>>(12)?,
                    owner_snapshot_digest: row.get::<_, Option<String>>(13)?,
                    decision_input_digest: row.get::<_, Option<String>>(14)?,
                    state: row.get::<_, String>(15)?,
                })
            },
        )
        .optional()
        .map_err(schema::map_sqlite)?;
    row.map(stored_invocation).transpose()
}

fn stored_invocation(
    row: RecipeInvocationRow,
) -> Result<StoredRecipeInvocation, ExecutionStoreError> {
    if row.runtime_map_profile != MAP_RECEIPT_PROFILE
        || row.schema_digest != crate::RUNTIME_MAP_SCHEMA_DIGEST
        || row.rpc_correlation_id != MAP_RPC_CORRELATION_ID
    {
        return Err(ExecutionStoreError::Corrupt);
    }
    let context = RecipeInvocationContext {
        runtime_config_digest: row.runtime_config_digest,
        instance_id: row.instance_id,
        gateway_session_id: row.gateway_session_id,
        mcp_session_id: row.mcp_session_id,
        lease_id: row.lease_id,
        lease_epoch: u64::try_from(row.lease_epoch).map_err(|_| ExecutionStoreError::Corrupt)?,
        state_id: row.state_id,
        generation: u64::try_from(row.generation).map_err(|_| ExecutionStoreError::Corrupt)?,
    };
    let stored = StoredRecipeInvocation {
        context,
        request_digest: row.request_digest,
        typed_result_digest: row.typed_result_digest,
        owner_snapshot_digest: row.owner_snapshot_digest,
        decision_input_digest: row.decision_input_digest,
        state: RecipeInvocationStatus::parse(&row.state)?,
    };
    if !valid_digest(&stored.request_digest)
        || stored
            .typed_result_digest
            .as_deref()
            .is_some_and(|digest| !valid_digest(digest))
        || stored
            .owner_snapshot_digest
            .as_deref()
            .is_some_and(|digest| !valid_digest(digest))
        || stored
            .decision_input_digest
            .as_deref()
            .is_some_and(|digest| !valid_digest(digest))
        || !matches_state_fields(&stored)
    {
        return Err(ExecutionStoreError::Corrupt);
    }
    Ok(stored)
}

fn matches_state_fields(stored: &StoredRecipeInvocation) -> bool {
    match stored.state {
        RecipeInvocationStatus::IntentRecorded => {
            stored.typed_result_digest.is_none()
                && stored.owner_snapshot_digest.is_none()
                && stored.decision_input_digest.is_none()
        }
        RecipeInvocationStatus::ResponseValidated => {
            stored.typed_result_digest.is_some()
                && stored.owner_snapshot_digest.is_none()
                && stored.decision_input_digest.is_none()
        }
        RecipeInvocationStatus::ContextValidated => {
            stored.typed_result_digest.is_some()
                && stored.owner_snapshot_digest.is_some()
                && stored.decision_input_digest.is_some()
        }
    }
}

pub(super) fn update_response(
    transaction: &rusqlite::Transaction<'_>,
    binding: &RecipeInvocationBinding,
    typed_result_digest: &str,
    now: i64,
) -> Result<(), ExecutionStoreError> {
    let changed = transaction
        .execute(
            "UPDATE recipe_map_invocations
             SET typed_result_digest = ?1, state = 'response_validated', updated_at = ?2
             WHERE run_id = ?3 AND episode_id = ?4 AND attempt_id = ?5 AND trajectory_id = ?6
               AND model_execution_id = ?7 AND recipe_id = ?8 AND recipe_revision = ?9
               AND operation = ?10 AND state = 'intent_recorded'",
            params![
                typed_result_digest,
                now,
                binding.lineage.run_id,
                binding.lineage.episode_id,
                binding.lineage.attempt_id,
                binding.lineage.trajectory_id,
                binding.model_execution_id,
                MAP_RECIPE_ID,
                MAP_RECIPE_REVISION,
                MAP_RECIPE_OPERATION,
            ],
        )
        .map_err(schema::map_sqlite)?;
    if changed != 1 {
        return Err(ExecutionStoreError::Conflict);
    }
    Ok(())
}

pub(super) fn update_context(
    transaction: &rusqlite::Transaction<'_>,
    binding: &RecipeInvocationBinding,
    owner_snapshot_digest: &str,
    decision_input_digest: &str,
    now: i64,
) -> Result<(), ExecutionStoreError> {
    let changed = transaction
        .execute(
            "UPDATE recipe_map_invocations
             SET owner_snapshot_digest = ?1, decision_input_digest = ?2,
                 state = 'context_validated', updated_at = ?3
             WHERE run_id = ?4 AND episode_id = ?5 AND attempt_id = ?6 AND trajectory_id = ?7
               AND model_execution_id = ?8 AND recipe_id = ?9 AND recipe_revision = ?10
               AND operation = ?11 AND state = 'response_validated'",
            params![
                owner_snapshot_digest,
                decision_input_digest,
                now,
                binding.lineage.run_id,
                binding.lineage.episode_id,
                binding.lineage.attempt_id,
                binding.lineage.trajectory_id,
                binding.model_execution_id,
                MAP_RECIPE_ID,
                MAP_RECIPE_REVISION,
                MAP_RECIPE_OPERATION,
            ],
        )
        .map_err(schema::map_sqlite)?;
    if changed != 1 {
        return Err(ExecutionStoreError::Conflict);
    }
    Ok(())
}
