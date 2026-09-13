// SPDX-License-Identifier: MIT

use rusqlite::{OptionalExtension, Transaction, params};

use super::store::{
    EventInput, SqliteBranchStore, append_event, digest_create, existing_operation,
    insert_artifacts, now_millis, record_operation,
};
use super::validation::validate_label;
use super::{
    BranchStoreError, DurableBranch, DurableBranchDraft, DurableBranchStatus, MAX_BRANCHES,
    MAX_TRANSITION_LABEL_BYTES, OccurrenceId,
};

fn ensure_experiment_and_parent(
    transaction: &Transaction<'_>,
    draft: &DurableBranchDraft,
) -> Result<(), BranchStoreError> {
    let existing: Option<String> = transaction
        .query_row(
            "SELECT root_branch_id FROM branch_experiments WHERE experiment_id = ?1",
            [&draft.experiment_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(BranchStoreError::persistence)?;
    match draft.parent_branch_id.as_deref() {
        None => {
            if existing.is_some() {
                return Err(BranchStoreError::Duplicate);
            }
            transaction
                .execute(
                    "INSERT INTO branch_experiments(
                        experiment_id, root_branch_id, created_at, updated_at
                     ) VALUES (?1, ?2, ?3, ?3)",
                    params![draft.experiment_id, draft.root_branch_id, now_millis()?],
                )
                .map_err(super::store::map_insert_error)?;
        }
        Some(parent) => {
            let Some(root) = existing else {
                return Err(BranchStoreError::UnknownExperiment);
            };
            if root != draft.root_branch_id {
                return Err(BranchStoreError::InvalidInput);
            }
            let parent_exists: bool = transaction
                .query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM durable_branches
                        WHERE experiment_id = ?1 AND branch_id = ?2
                    )",
                    params![draft.experiment_id, parent],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(BranchStoreError::persistence)?
                != 0;
            if !parent_exists {
                return Err(BranchStoreError::UnknownParent);
            }
        }
    }
    Ok(())
}

fn ensure_occurrence(
    transaction: &Transaction<'_>,
    draft: &DurableBranchDraft,
) -> Result<(), BranchStoreError> {
    if let Some(parent) = &draft.fork.parent_occurrence_id {
        let exists: bool = transaction
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM branch_occurrences
                    WHERE experiment_id = ?1 AND occurrence_id = ?2
                )",
                params![draft.experiment_id, parent.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .map_err(BranchStoreError::persistence)?
            != 0;
        if !exists {
            return Err(BranchStoreError::UnknownParent);
        }
    }
    let existing: Option<(Option<String>, String)> = transaction
        .query_row(
            "SELECT parent_occurrence_id, state_digest FROM branch_occurrences
             WHERE experiment_id = ?1 AND occurrence_id = ?2",
            params![draft.experiment_id, draft.fork.occurrence_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(BranchStoreError::persistence)?;
    if let Some((parent, state)) = existing {
        let expected_parent = draft
            .fork
            .parent_occurrence_id
            .as_ref()
            .map(|value| value.as_str());
        if parent.as_deref() != expected_parent || state != draft.fork.state_digest.as_str() {
            return Err(BranchStoreError::Duplicate);
        }
        return Ok(());
    }
    transaction
        .execute(
            "INSERT INTO branch_occurrences(
                experiment_id, occurrence_id, parent_occurrence_id, state_digest, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                draft.experiment_id,
                draft.fork.occurrence_id.as_str(),
                draft
                    .fork
                    .parent_occurrence_id
                    .as_ref()
                    .map(OccurrenceId::as_str),
                draft.fork.state_digest.as_str(),
                now_millis()?
            ],
        )
        .map_err(super::store::map_insert_error)?;
    Ok(())
}

impl SqliteBranchStore {
    /// Atomically creates an experiment root or immutable child branch.
    pub fn create(
        &self,
        operation_id: &str,
        draft: DurableBranchDraft,
    ) -> Result<DurableBranch, BranchStoreError> {
        validate_label(operation_id, MAX_TRANSITION_LABEL_BYTES)?;
        draft.validate()?;
        let digest = digest_create(&draft);
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction()
            .map_err(BranchStoreError::persistence)?;
        if let Some(existing) = existing_operation(&transaction, operation_id, &digest)? {
            transaction
                .commit()
                .map_err(BranchStoreError::persistence)?;
            drop(connection);
            return Ok(existing);
        }
        ensure_experiment_and_parent(&transaction, &draft)?;
        let count: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM durable_branches WHERE experiment_id = ?1",
                [&draft.experiment_id],
                |row| row.get(0),
            )
            .map_err(BranchStoreError::persistence)?;
        if count >= MAX_BRANCHES as i64 {
            return Err(BranchStoreError::Capacity);
        }
        ensure_occurrence(&transaction, &draft)?;
        let now = now_millis()?;
        transaction
            .execute(
                "INSERT INTO durable_branches(
                    experiment_id, root_branch_id, branch_id, parent_branch_id,
                    fork_occurrence_id, strategy, source_handle, trajectory_prefix,
                    effective_seed, setup_digest, boundary, assurance, run_id,
                    episode_id, trajectory_id, context_id, policy_revision, config_revision,
                    name, notes, status, metadata_revision, created_at, updated_at
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'unverified', ?12,
                    ?13, ?14, ?15, ?16, ?17, ?18, ?19, 'pending', 0, ?20, ?20
                 )",
                params![
                    draft.experiment_id,
                    draft.root_branch_id,
                    draft.branch_id,
                    draft.parent_branch_id,
                    draft.fork.occurrence_id.as_str(),
                    super::store::strategy_label(draft.strategy),
                    draft.source_handle,
                    draft.trajectory_prefix,
                    draft.effective_seed,
                    draft.setup_digest,
                    draft.boundary,
                    draft.run_id,
                    draft.episode_id,
                    draft.trajectory_id,
                    draft.context_id,
                    draft.policy_revision,
                    draft.config_revision,
                    draft.name,
                    draft.notes,
                    now
                ],
            )
            .map_err(super::store::map_insert_error)?;
        record_operation(
            &transaction,
            operation_id,
            "create",
            &digest,
            &draft.experiment_id,
            &draft.branch_id,
            now,
        )?;
        insert_artifacts(
            &transaction,
            &draft.experiment_id,
            &draft.branch_id,
            operation_id,
            &draft.artifacts,
        )?;
        append_event(
            &transaction,
            EventInput {
                experiment_id: &draft.experiment_id,
                branch_id: &draft.branch_id,
                operation_id,
                kind: "created",
                status: DurableBranchStatus::Pending,
                metadata_revision: 0,
                now,
            },
        )?;
        transaction
            .commit()
            .map_err(BranchStoreError::persistence)?;
        drop(connection);
        self.get(&draft.experiment_id, &draft.branch_id)?
            .ok_or(BranchStoreError::Corrupt)
    }
}
