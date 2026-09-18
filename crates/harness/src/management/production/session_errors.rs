// SPDX-License-Identifier: MIT

//! Error mapping for the served-live session boundary.

use super::ManagementError;

pub(crate) fn runtime_error(
    code: &'static str,
) -> impl FnOnce(crate::PortError) -> ManagementError {
    move |error| ManagementError::unavailable(code, error.to_string())
}

pub(crate) fn provider_error(error: crate::episode::PolicyError) -> ManagementError {
    if let crate::episode::PolicyError::SelectedContextLimit(limit) = error {
        return ManagementError::capability(
            "context_render_limit_exceeded",
            format!("prepared context exceeds the selected owner limit: {limit}"),
        );
    }
    ManagementError::unavailable("provider_decision_failed", error.to_string())
}
