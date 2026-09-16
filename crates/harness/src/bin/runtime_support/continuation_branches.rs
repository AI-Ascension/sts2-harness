// SPDX-License-Identifier: MIT

//! Runtime startup reconciliation for durable continuation branches.
//!
//! The production runtime binary is the owner-side consumer for durable continuation branches: it
//! constructs the branch store, resolves every pending or in-progress preparation branch for its
//! experiment scope before any episode is admitted or resumed, and reads each resolved branch back
//! to confirm the durable strategy descriptor survived the transition. `running` branches are not
//! rewritten here because the runtime cannot prove whether another process still owns their
//! destination lease; explicit continuation admission refuses them until owner reconciliation.
//! Reconciliation never retries an uncertain effect and never allocates a destination.
//!
//! The functions in this module take plain parameters so the same production source is exercised
//! directly by `tests/durable_branch_consumer_roundtrip.rs`; the binary resolves the store path and
//! experiment scope from its own configuration.

use std::path::Path;

use sts2_harness::{DurableBranch, SqliteBranchStore};

/// Deterministic operation prefix for the runtime's idempotent startup reconciliation.
pub(crate) const STARTUP_RECONCILE_OPERATION_PREFIX: &str = "runtime-continuation-startup";

/// Resolves every pending, restoring, replaying, or unknown branch for `experiment_id`.
///
/// Opens the durable branch store at `store_path`, applies the owner startup reconciliation, and
/// returns the persisted records re-read from the store. A branch whose immutable continuation
/// identity or strategy descriptor changed during reconciliation is a hard error: the runtime must
/// never continue on a descriptor the store did not preserve.
pub(crate) fn reconcile_continuation_branches(
    store_path: &Path,
    operation_prefix: &str,
    experiment_id: &str,
) -> Result<Vec<DurableBranch>, String> {
    let store = SqliteBranchStore::open(store_path)
        .map_err(|error| format!("cannot open the durable continuation branch store: {error}"))?;
    let reconciled = store
        .reconcile_startup(operation_prefix, experiment_id)
        .map_err(|error| format!("cannot reconcile durable continuation branches: {error}"))?;
    let mut persisted = Vec::with_capacity(reconciled.len());
    for resolved in reconciled {
        let stored = store
            .get(experiment_id, &resolved.branch_id)
            .map_err(|error| format!("cannot read back reconciled continuation branch: {error}"))?
            .ok_or_else(|| String::from("reconciled continuation branch disappeared"))?;
        if !descriptor_preserved(&resolved, &stored) {
            return Err(format!(
                "reconciled continuation branch {} lost its strategy descriptor",
                resolved.branch_id
            ));
        }
        persisted.push(stored);
    }
    Ok(persisted)
}

/// Compares the immutable continuation identity and strategy descriptor of two records.
///
/// Lifecycle status, CAS revision, and timestamps are expected to change during reconciliation;
/// branch identity, strategy descriptor, fork source, associations, and digests must not.
pub(crate) fn descriptor_preserved(expected: &DurableBranch, persisted: &DurableBranch) -> bool {
    expected.experiment_id == persisted.experiment_id
        && expected.root_branch_id == persisted.root_branch_id
        && expected.branch_id == persisted.branch_id
        && expected.parent_branch_id == persisted.parent_branch_id
        && expected.fork == persisted.fork
        && expected.strategy == persisted.strategy
        && expected.source_handle == persisted.source_handle
        && expected.trajectory_prefix == persisted.trajectory_prefix
        && expected.effective_seed == persisted.effective_seed
        && expected.setup_digest == persisted.setup_digest
        && expected.boundary == persisted.boundary
        && expected.assurance == persisted.assurance
        && expected.run_id == persisted.run_id
        && expected.episode_id == persisted.episode_id
        && expected.trajectory_id == persisted.trajectory_id
        && expected.context_id == persisted.context_id
        && expected.policy_revision == persisted.policy_revision
        && expected.config_revision == persisted.config_revision
        && expected.name == persisted.name
        && expected.notes == persisted.notes
        && expected.artifacts == persisted.artifacts
}
