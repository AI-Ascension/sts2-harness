// SPDX-License-Identifier: MIT

//! Error translation shared by the synthetic ports.
//!
//! Part of the `workflow_ports` split. Refs sts2-harness#570.

use super::service::ManagementError;
use crate::workflow::RuntimeFault;

pub(super) fn runtime_error(error: RuntimeFault) -> ManagementError {
    match error {
        RuntimeFault::BudgetExceeded => {
            ManagementError::budget("runtime_budget_exhausted", error.to_string())
        }
        RuntimeFault::UnknownEffect => ManagementError::unresolved(
            "unknown_effect",
            "synthetic runtime reported an unresolved effect",
        ),
        RuntimeFault::InvalidState => ManagementError::conflict("runtime_state", error.to_string()),
        _ => ManagementError::unavailable("runtime_failure", error.to_string()),
    }
}

pub(super) fn runtime_store_error(error: super::store::StoreError) -> ManagementError {
    ManagementError::store(error.code, error.message)
}

pub(super) fn lock_error<T>(_: std::sync::PoisonError<T>) -> ManagementError {
    ManagementError::store("runtime_lock", "synthetic runtime lock is poisoned")
}
