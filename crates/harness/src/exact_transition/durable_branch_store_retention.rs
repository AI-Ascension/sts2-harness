// SPDX-License-Identifier: MIT

use std::collections::BTreeSet;

use rusqlite::{Transaction, params};

use super::store::{
    EventInput, SqliteBranchStore, append_event, begin_write_transaction, digest_fields,
    existing_operation, now_millis,
};
use super::store_prune_plan::{insert_prune_plan, load_prune_plan};
use super::store_reachability::{from_connection, from_transaction};
use super::validation::validate_label;
use super::{
    BranchArtifactReference, BranchStoreError, DurableBranch, DurableBranchStatus, MAX_BRANCH_PAGE,
    MAX_TRANSITION_LABEL_BYTES,
};

/// Operator-selected retention policy for explicit branch pruning.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BranchRetentionPolicy {
    /// Keep completed branches unless an operator explicitly disables this protection.
    pub retain_completed: bool,
    /// Do not prune a branch until this many milliseconds have elapsed since its last update.
    pub minimum_age_millis: u64,
}

impl Default for BranchRetentionPolicy {
    fn default() -> Self {
        Self {
            retain_completed: true,
            minimum_age_millis: 0,
        }
    }
}

/// Explicit branch IDs selected for a preview or prune operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BranchPruneRequest {
    /// Experiment namespace.
    pub experiment_id: String,
    /// Archived (or policy-eligible completed) branches to tombstone.
    pub branch_ids: Vec<String>,
    /// Retention policy applied to the selection.
    pub policy: BranchRetentionPolicy,
}

/// Reference-aware prune result. Blob deletion remains the artifact owner's responsibility.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BranchPrunePlan {
    /// Branch metadata rows selected for tombstoning.
    pub branch_ids: Vec<String>,
    /// Artifact references still reachable from non-pruned branches.
    pub retained_artifacts: Vec<BranchArtifactReference>,
    /// Artifact references with no remaining branch root reference.
    pub collectable_artifacts: Vec<BranchArtifactReference>,
}

impl SqliteBranchStore {
    /// Computes a reference-aware prune preview without changing durable state.
    pub fn prune_preview(
        &self,
        request: &BranchPruneRequest,
    ) -> Result<BranchPrunePlan, BranchStoreError> {
        validate_request(request)?;
        let connection = self.lock()?;
        let branches = super::store_reads::all_branches(&connection, &request.experiment_id)?;
        let selected = eligible_branches(&branches, request)?;
        let refs = from_connection(&connection, &request.experiment_id, &selected)?;
        Ok(BranchPrunePlan {
            branch_ids: selected,
            retained_artifacts: refs.retained,
            collectable_artifacts: refs.collectable,
        })
    }

    /// Tombstones selected branch metadata and collectable artifact edges atomically.
    pub fn prune(
        &self,
        operation_id: &str,
        request: &BranchPruneRequest,
    ) -> Result<BranchPrunePlan, BranchStoreError> {
        validate_label(operation_id, MAX_TRANSITION_LABEL_BYTES)?;
        validate_request(request)?;
        let mut connection = self.lock()?;
        let transaction = begin_write_transaction(&mut connection)?;
        let digest = digest_prune(request);
        if existing_operation(&transaction, operation_id, &digest)?.is_some() {
            let plan =
                load_prune_plan(&transaction, operation_id)?.ok_or(BranchStoreError::Corrupt)?;
            transaction
                .commit()
                .map_err(BranchStoreError::persistence)?;
            return Ok(plan);
        }
        let branches = super::store_reads::all_branches_tx(&transaction, &request.experiment_id)?;
        let selected = eligible_branches(&branches, request)?;
        let refs = from_transaction(&transaction, &request.experiment_id, &selected)?;
        let now = now_millis()?;
        let operation_branch = selected.first().ok_or(BranchStoreError::InvalidInput)?;
        for branch_id in &selected {
            let already_tombstoned: bool = transaction
                .query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM branch_tombstones
                        WHERE experiment_id = ?1 AND branch_id = ?2
                    )",
                    params![request.experiment_id, branch_id],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(BranchStoreError::persistence)?
                != 0;
            if already_tombstoned {
                continue;
            }
            transaction
                .execute(
                    "INSERT INTO branch_tombstones(
                        experiment_id, branch_id, operation_id, reason, pruned_at
                     ) VALUES (?1, ?2, ?3, 'explicit_prune', ?4)",
                    params![request.experiment_id, branch_id, operation_id, now],
                )
                .map_err(BranchStoreError::persistence)?;
            for reference in branches
                .iter()
                .find(|value| value.branch_id == *branch_id)
                .map(|value| value.artifacts.as_slice())
                .unwrap_or(&[])
            {
                let collectable = refs.collectable.contains(reference);
                transaction
                    .execute(
                        "INSERT INTO branch_tombstone_artifacts(
                            experiment_id, branch_id, artifact_id, role, collectable
                         ) VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![
                            request.experiment_id,
                            branch_id,
                            reference.artifact_id,
                            reference.role.as_str(),
                            if collectable { 1_i64 } else { 0_i64 }
                        ],
                    )
                    .map_err(BranchStoreError::persistence)?;
            }
            transaction
                .execute(
                    "UPDATE branch_artifacts SET tombstoned = 1
                     WHERE experiment_id = ?1 AND branch_id = ?2",
                    params![request.experiment_id, branch_id],
                )
                .map_err(BranchStoreError::persistence)?;
            let branch = branches
                .iter()
                .find(|value| value.branch_id == *branch_id)
                .ok_or(BranchStoreError::Corrupt)?;
            append_event(
                &transaction,
                EventInput {
                    experiment_id: &request.experiment_id,
                    branch_id,
                    operation_id,
                    kind: "pruned",
                    status: branch.status,
                    metadata_revision: branch.metadata_revision,
                    now,
                },
            )?;
        }
        record_prune_operation(
            &transaction,
            operation_id,
            &digest,
            &request.experiment_id,
            operation_branch,
            now,
        )?;
        let plan = BranchPrunePlan {
            branch_ids: selected,
            retained_artifacts: refs.retained,
            collectable_artifacts: refs.collectable,
        };
        insert_prune_plan(&transaction, operation_id, &request.experiment_id, &plan)?;
        transaction
            .commit()
            .map_err(BranchStoreError::persistence)?;
        Ok(plan)
    }
}

fn validate_request(request: &BranchPruneRequest) -> Result<(), BranchStoreError> {
    validate_label(&request.experiment_id, MAX_TRANSITION_LABEL_BYTES)?;
    if request.branch_ids.is_empty() || request.branch_ids.len() > MAX_BRANCH_PAGE as usize {
        return Err(BranchStoreError::InvalidInput);
    }
    let mut unique = BTreeSet::new();
    for branch_id in &request.branch_ids {
        validate_label(branch_id, MAX_TRANSITION_LABEL_BYTES)?;
        if !unique.insert(branch_id) {
            return Err(BranchStoreError::Duplicate);
        }
    }
    Ok(())
}

fn eligible_branches(
    branches: &[DurableBranch],
    request: &BranchPruneRequest,
) -> Result<Vec<String>, BranchStoreError> {
    let now = now_millis()?;
    let mut selected = Vec::with_capacity(request.branch_ids.len());
    for branch_id in &request.branch_ids {
        let branch = branches
            .iter()
            .find(|value| &value.branch_id == branch_id)
            .ok_or(BranchStoreError::UnknownBranch)?;
        let age = now.saturating_sub(branch.updated_at).max(0) as u64;
        if age < request.policy.minimum_age_millis {
            return Err(BranchStoreError::InvalidTransition);
        }
        let allowed = branch.status == DurableBranchStatus::Archived
            || (branch.status == DurableBranchStatus::Completed
                && !request.policy.retain_completed);
        if !allowed {
            return Err(BranchStoreError::InvalidTransition);
        }
        selected.push(branch.branch_id.clone());
    }
    selected.sort();
    Ok(selected)
}

fn digest_prune(request: &BranchPruneRequest) -> String {
    let mut fields = vec![
        "prune".to_owned(),
        request.experiment_id.clone(),
        request.policy.retain_completed.to_string(),
        request.policy.minimum_age_millis.to_string(),
    ];
    let mut branch_ids = request.branch_ids.clone();
    branch_ids.sort();
    fields.extend(branch_ids);
    digest_fields(fields)
}

fn record_prune_operation(
    transaction: &Transaction<'_>,
    operation_id: &str,
    digest: &str,
    experiment_id: &str,
    branch_id: &str,
    now: i64,
) -> Result<(), BranchStoreError> {
    transaction
        .execute(
            "INSERT INTO branch_operations(
                operation_id, operation_kind, payload_digest, experiment_id, branch_id, created_at
             ) VALUES (?1, 'prune', ?2, ?3, ?4, ?5)",
            params![operation_id, digest, experiment_id, branch_id, now],
        )
        .map_err(BranchStoreError::persistence)?;
    Ok(())
}
