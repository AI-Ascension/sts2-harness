// SPDX-License-Identifier: MIT

//! The one durable-store owner shared by the worker control and execution paths.

use std::sync::{Arc, Mutex, MutexGuard, TryLockError};

use sts2_harness::{
    ExecutionFingerprint, ExecutionLineage, ExecutionStore, ResumeState, StoredEpisode,
};

pub(super) type SharedExecutionStore = Arc<Mutex<ExecutionStore>>;

pub(super) fn share_store(store: ExecutionStore) -> SharedExecutionStore {
    Arc::new(Mutex::new(store))
}

/// The durable identity and resume boundary observed during one short store lease.
///
/// A runtime handle must not assemble these values from separate leases: another shared path may
/// advance the current attempt or checkpoint between those reads. Keeping the episode, resume
/// state, and checkpoint in one snapshot lets callers reject a stale or cross-lineage handle
/// before it can perform an effect.
pub(super) struct StoreSnapshot {
    pub(super) episode: StoredEpisode,
    pub(super) resume: ResumeState,
}

pub(super) fn snapshot(
    store: &ExecutionStore,
    lineage: &ExecutionLineage,
    fingerprint: &ExecutionFingerprint,
) -> Result<StoreSnapshot, String> {
    let resume = store
        .resume_episode(&lineage.episode_id, fingerprint)
        .map_err(|error| format!("cannot inspect runtime-v3 execution state: {error}"))?;
    if matches!(resume, ResumeState::New) {
        return Err(String::from(
            "runtime-v3 execution episode is missing from the admitted store",
        ));
    }
    let episode = store
        .load_episode(&lineage.episode_id)
        .map_err(|error| format!("cannot load runtime-v3 execution episode: {error}"))?;
    if episode.lineage != *lineage {
        return Err(String::from(
            "runtime-v3 stored episode lineage does not match the approved runtime",
        ));
    }
    if episode.fingerprint != *fingerprint {
        return Err(String::from(
            "runtime-v3 stored episode fingerprint does not match the approved runtime",
        ));
    }
    if let ResumeState::Ready { checkpoint, .. } = &resume
        && checkpoint.as_deref() != episode.last_checkpoint.as_ref()
    {
        return Err(String::from(
            "runtime-v3 stored resume boundary does not match the latest checkpoint",
        ));
    }
    Ok(StoreSnapshot { episode, resume })
}

/// Takes a bounded, non-blocking store lease. The lease must never span provider, MCP, or other
/// external I/O; callers hold it only for one short durable operation.
pub(super) fn try_lock(
    store: &SharedExecutionStore,
) -> Result<MutexGuard<'_, ExecutionStore>, String> {
    match store.try_lock() {
        Ok(guard) => Ok(guard),
        Err(TryLockError::WouldBlock) => Err(String::from(
            "runtime-v3 execution store is busy; retry the durable operation",
        )),
        Err(TryLockError::Poisoned(_)) => {
            Err(String::from("runtime-v3 execution store is poisoned"))
        }
    }
}
