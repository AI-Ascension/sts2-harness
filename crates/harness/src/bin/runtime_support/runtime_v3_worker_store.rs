// SPDX-License-Identifier: MIT

//! The one durable-store owner shared by the worker control and execution paths.

use std::sync::{Arc, Mutex, MutexGuard, TryLockError};

use sts2_harness::ExecutionStore;

pub(super) type SharedExecutionStore = Arc<Mutex<ExecutionStore>>;

pub(super) fn share_store(store: ExecutionStore) -> SharedExecutionStore {
    Arc::new(Mutex::new(store))
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
