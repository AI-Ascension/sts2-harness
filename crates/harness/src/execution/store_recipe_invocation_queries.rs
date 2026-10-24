// SPDX-License-Identifier: MIT

use rusqlite::{Connection, OptionalExtension, params};

use super::super::schema;
use super::super::types::recipe_invocation::{
    MAP_RECEIPT_PROFILE, MAP_RECIPE_ID, MAP_RECIPE_OPERATION, MAP_RECIPE_REVISION,
    MAP_RPC_CORRELATION_ID, RecipeInvocationBinding, RecipeInvocationContext,
    RecipeInvocationStatus,
};
use super::super::types::{ExecutionStoreError, valid_digest};
use super::StoredRecipeInvocation;

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
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, Option<String>>(12)?,
                    row.get::<_, Option<String>>(13)?,
                    row.get::<_, Option<String>>(14)?,
                    row.get::<_, String>(15)?,
                ))
            },
        )
        .optional()
        .map_err(schema::map_sqlite)?;
    row.map(stored_invocation).transpose()
}

fn stored_invocation(
    row: (
        String,
        String,
        String,
        String,
        String,
        i64,
        String,
        i64,
        String,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
    ),
) -> Result<StoredRecipeInvocation, ExecutionStoreError> {
    if row.8 != MAP_RECEIPT_PROFILE
        || row.9 != crate::RUNTIME_MAP_SCHEMA_DIGEST
        || row.10 != MAP_RPC_CORRELATION_ID
    {
        return Err(ExecutionStoreError::Corrupt);
    }
    let context = RecipeInvocationContext {
        runtime_config_digest: row.0,
        instance_id: row.1,
        gateway_session_id: row.2,
        mcp_session_id: row.3,
        lease_id: row.4,
        lease_epoch: u64::try_from(row.5).map_err(|_| ExecutionStoreError::Corrupt)?,
        state_id: row.6,
        generation: u64::try_from(row.7).map_err(|_| ExecutionStoreError::Corrupt)?,
    };
    let stored = StoredRecipeInvocation {
        context,
        request_digest: row.11,
        typed_result_digest: row.12,
        owner_snapshot_digest: row.13,
        decision_input_digest: row.14,
        state: RecipeInvocationStatus::parse(&row.15)?,
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
