// SPDX-License-Identifier: MIT

use super::schema;
use super::store_core::{ExecutionStore, ensure_current_lineage};
use super::types::recipe_invocation::{
    RecipeInvocationBinding, RecipeInvocationContext, RecipeInvocationReceipt,
    RecipeInvocationStatus,
};
use crate::identity::ModelExecutionId;

use super::types::{ExecutionLineage, ExecutionStoreError, valid_digest};

#[path = "store_recipe_invocation_queries.rs"]
mod queries;

const MAX_RECIPE_INVOCATIONS: i64 = 4_096;

struct StoredRecipeInvocation {
    context: RecipeInvocationContext,
    request_digest: String,
    typed_result_digest: Option<String>,
    owner_snapshot_digest: Option<String>,
    decision_input_digest: Option<String>,
    state: RecipeInvocationStatus,
}

impl ExecutionStore {
    /// Reads receipt state and digests for one exact, current invocation binding.
    ///
    /// The returned view contains no response payload or database access.
    pub fn recipe_invocation_receipt(
        &mut self,
        binding: &RecipeInvocationBinding,
    ) -> Result<Option<RecipeInvocationReceipt>, ExecutionStoreError> {
        self.ensure_open()?;
        binding.validate()?;
        let request_digest = binding.request_digest();
        let tx = schema::transaction(&mut self.connection)?;
        ensure_current_lineage(&tx, &binding.lineage)?;
        let receipt = match queries::load_invocation(&tx, binding)? {
            Some(stored) => {
                ensure_same_binding(&stored, binding, &request_digest)?;
                Some(RecipeInvocationReceipt::new(
                    stored.state,
                    stored.request_digest,
                    stored.typed_result_digest,
                    stored.owner_snapshot_digest,
                    stored.decision_input_digest,
                ))
            }
            None => None,
        };
        tx.commit().map_err(schema::map_sqlite)?;
        Ok(receipt)
    }

    /// Checks for a receipt for one exact current lineage and model execution.
    ///
    /// This metadata-only query exposes no receipt payload or binding fields.
    pub fn recipe_invocation_exists_for_execution(
        &mut self,
        lineage: &ExecutionLineage,
        model_execution_id: ModelExecutionId,
    ) -> Result<bool, ExecutionStoreError> {
        self.ensure_open()?;
        let tx = schema::transaction(&mut self.connection)?;
        ensure_current_lineage(&tx, lineage)?;
        let exists = queries::invocation_exists_for_execution(&tx, lineage, model_execution_id)?;
        tx.commit().map_err(schema::map_sqlite)?;
        Ok(exists)
    }

    /// Commits the immutable map invocation before its one read can start.
    pub fn record_recipe_invocation_intent(
        &mut self,
        binding: &RecipeInvocationBinding,
    ) -> Result<bool, ExecutionStoreError> {
        self.ensure_open()?;
        binding.validate()?;
        let request_digest = binding.request_digest();
        let now = ExecutionStore::now();
        let tx = schema::transaction(&mut self.connection)?;
        ensure_current_lineage(&tx, &binding.lineage)?;
        if let Some(existing) = queries::load_invocation(&tx, binding)? {
            ensure_same_binding(&existing, binding, &request_digest)?;
            if existing.state == RecipeInvocationStatus::IntentRecorded {
                tx.commit().map_err(schema::map_sqlite)?;
                return Ok(false);
            }
            return Err(ExecutionStoreError::Conflict);
        }
        let count = tx
            .query_row("SELECT COUNT(*) FROM recipe_map_invocations", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(schema::map_sqlite)?;
        if count >= MAX_RECIPE_INVOCATIONS {
            return Err(ExecutionStoreError::Capacity);
        }
        queries::insert_intent(&tx, binding, &request_digest, now)?;
        tx.commit().map_err(schema::map_sqlite)?;
        Ok(true)
    }

    /// Records the typed protocol result digest after the existing reader accepted it.
    pub fn record_recipe_invocation_response(
        &mut self,
        binding: &RecipeInvocationBinding,
        typed_result_digest: &str,
    ) -> Result<(), ExecutionStoreError> {
        self.ensure_open()?;
        binding.validate()?;
        if !valid_digest(typed_result_digest) {
            return Err(ExecutionStoreError::InvalidOperation);
        }
        let request_digest = binding.request_digest();
        let now = ExecutionStore::now();
        let tx = schema::transaction(&mut self.connection)?;
        ensure_current_lineage(&tx, &binding.lineage)?;
        let existing =
            queries::load_invocation(&tx, binding)?.ok_or(ExecutionStoreError::Missing)?;
        ensure_same_binding(&existing, binding, &request_digest)?;
        match existing.state {
            RecipeInvocationStatus::IntentRecorded => {
                queries::update_response(&tx, binding, typed_result_digest, now)?;
            }
            RecipeInvocationStatus::ResponseValidated
            | RecipeInvocationStatus::ContextValidated
                if existing.typed_result_digest.as_deref() == Some(typed_result_digest) => {}
            _ => return Err(ExecutionStoreError::Conflict),
        }
        tx.commit().map_err(schema::map_sqlite)
    }

    /// Finalizes only the response whose validated owner context matches this invocation.
    pub fn finalize_recipe_invocation(
        &mut self,
        binding: &RecipeInvocationBinding,
        owner_snapshot_digest: &str,
        decision_input_digest: &str,
    ) -> Result<(), ExecutionStoreError> {
        self.ensure_open()?;
        binding.validate()?;
        if !valid_digest(owner_snapshot_digest) || !valid_digest(decision_input_digest) {
            return Err(ExecutionStoreError::InvalidOperation);
        }
        let request_digest = binding.request_digest();
        let now = ExecutionStore::now();
        let tx = schema::transaction(&mut self.connection)?;
        ensure_current_lineage(&tx, &binding.lineage)?;
        let existing =
            queries::load_invocation(&tx, binding)?.ok_or(ExecutionStoreError::Missing)?;
        ensure_same_binding(&existing, binding, &request_digest)?;
        match existing.state {
            RecipeInvocationStatus::ResponseValidated => {
                queries::update_context(
                    &tx,
                    binding,
                    owner_snapshot_digest,
                    decision_input_digest,
                    now,
                )?;
            }
            RecipeInvocationStatus::ContextValidated
                if existing.owner_snapshot_digest.as_deref() == Some(owner_snapshot_digest)
                    && existing.decision_input_digest.as_deref() == Some(decision_input_digest) => {
            }
            _ => return Err(ExecutionStoreError::Conflict),
        }
        tx.commit().map_err(schema::map_sqlite)
    }
}

fn ensure_same_binding(
    stored: &StoredRecipeInvocation,
    binding: &RecipeInvocationBinding,
    request_digest: &str,
) -> Result<(), ExecutionStoreError> {
    if stored.context != binding.context || stored.request_digest != request_digest {
        return Err(ExecutionStoreError::Conflict);
    }
    Ok(())
}

#[cfg(test)]
#[path = "store_recipe_invocation_tests.rs"]
mod tests;
