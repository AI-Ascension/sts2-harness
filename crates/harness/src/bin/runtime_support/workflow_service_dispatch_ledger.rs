// SPDX-License-Identifier: MIT

//! The served composition's durable prepared-dispatch receipt store.
//!
//! Which durable store a served deployment commits its recorded receipts to is an owner decision
//! (`#398`), so the served binary attaches a store only when the operator names one and keeps its
//! session-lifetime ledger otherwise. This module owns that decision, so `workflow_service` only
//! builds the factory and asks for the store to attach.

use std::path::PathBuf;
use sts2_harness::context_capture::FileDispatchLedgerPort;

/// The store `STS2_WORKFLOW_DISPATCH_LEDGER` asks the served composition to attach, if any.
///
/// An explicitly empty value is refused rather than read as "no store", so a misconfigured
/// deployment cannot silently lose the once-only guarantee across a restart.
pub(super) fn port_from_environment() -> Result<Option<FileDispatchLedgerPort>, String> {
    Ok(path_from_environment()?.map(FileDispatchLedgerPort::open))
}

/// The image path named by the environment, when one is configured.
fn path_from_environment() -> Result<Option<PathBuf>, String> {
    match std::env::var("STS2_WORKFLOW_DISPATCH_LEDGER") {
        Ok(value) => path_from_value(Some(value)),
        Err(std::env::VarError::NotPresent) => path_from_value(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(String::from(
            "STS2_WORKFLOW_DISPATCH_LEDGER is not valid UTF-8",
        )),
    }
}

/// The ledger-path decision for one observed environment value.
fn path_from_value(value: Option<String>) -> Result<Option<PathBuf>, String> {
    match value {
        None => Ok(None),
        Some(value) if value.trim().is_empty() => Err(String::from(
            "STS2_WORKFLOW_DISPATCH_LEDGER must name a file path when set",
        )),
        Some(value) => Ok(Some(PathBuf::from(value))),
    }
}

#[cfg(test)]
#[path = "workflow_service_dispatch_ledger_tests.rs"]
mod tests;
