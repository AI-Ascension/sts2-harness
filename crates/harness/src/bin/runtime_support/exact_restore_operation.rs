// SPDX-License-Identifier: MIT

#[path = "exact_restore_operation_flow.rs"]
mod flow;
#[path = "exact_restore_operation_transport.rs"]
mod transport;

use serde_json::Value;

use super::{FailureSafety, canonical_bytes};
use crate::runtime_support::{
    branch_continuation_runtime::SelectedBranchContinuation, config::RuntimeConfig,
};

#[cfg(test)]
pub(super) use transport::{exchange, request_frame};

pub(crate) fn execute(
    selected: &SelectedBranchContinuation,
    config: &RuntimeConfig,
) -> Result<Vec<u8>, super::ExactRestoreError> {
    let closure = selected.exact_restore().ok_or_else(|| {
        failure(
            FailureSafety::NotStarted,
            "exact-restore source closure was not admitted",
        )
    })?;
    let (operation_id, expected_owner) = selected
        .exact_restore_owner()
        .map_err(|message| failure(FailureSafety::NotStarted, message))?;
    let operation_uuid = uuid::Uuid::parse_str(&operation_id).map_err(|_| {
        failure(
            FailureSafety::NotStarted,
            "persisted exact-restore operation id is invalid",
        )
    })?;
    if operation_uuid.get_version_num() != 4 {
        return Err(failure(
            FailureSafety::NotStarted,
            "persisted exact-restore operation id is not UUIDv4",
        ));
    }
    let mut process = None;
    let mut rpc_id = 3_u64;
    transport::ensure_profile(&mut process, config).map_err(uncertain)?;
    let result = flow::run_operation(
        &mut process,
        &mut rpc_id,
        config,
        selected,
        closure,
        &operation_id,
        &expected_owner,
    );
    let close = process.as_mut().map_or(Ok(()), |mcp| mcp.close());
    match (result, close) {
        (Ok(receipt), Ok(())) => {
            canonical_bytes(&receipt).map_err(|message| failure(FailureSafety::Uncertain, message))
        }
        (Err(mut error), Err(close_error)) => {
            error.message = format!(
                "{}; exact-restore MCP close failed: {close_error}",
                error.message
            );
            Err(error)
        }
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(close_error)) => Err(failure(
            FailureSafety::Uncertain,
            format!("verified receipt was retained but MCP shutdown failed: {close_error}"),
        )),
    }
}

pub(super) fn failure(
    safety: FailureSafety,
    message: impl Into<String>,
) -> super::ExactRestoreError {
    super::ExactRestoreError {
        safety,
        message: message.into(),
    }
}

pub(super) fn not_started(message: impl Into<String>) -> super::ExactRestoreError {
    failure(FailureSafety::NotStarted, message)
}

pub(super) fn uncertain(message: impl Into<String>) -> super::ExactRestoreError {
    failure(FailureSafety::Uncertain, message)
}

pub(super) fn phase_error(
    error: super::ExactRestoreError,
    phase: &str,
) -> super::ExactRestoreError {
    failure(
        error.safety,
        format!("exact-restore {phase}: {}", error.message),
    )
}

pub(super) fn is_error_response(frame: &Value) -> bool {
    frame["kind"] == "exact_restore_error_response"
}
