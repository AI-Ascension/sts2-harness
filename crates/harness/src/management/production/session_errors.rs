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

/// Re-classify a failure that happened at or after the provider exchange.
///
/// The held-attempt discipline releases a paid-decision identity only when the boundary owner
/// reports that the refusal happened *before* it could write. Only the code that owns the
/// boundary can tell those apart, so its error class is that report: `Unresolved` means the
/// exchange may already have happened.
///
/// The code alone is never proof. `context_render_source_unavailable`, for example, is raised by
/// `render_source_for_decision` before the exchange *and* by the post-exchange re-assertion, so a
/// caller that released the hold on that code would authorize a second paid exchange whenever the
/// store lost the source in between. The code is preserved for observability; only the class
/// changes.
pub(crate) fn exchange_unresolved(error: ManagementError) -> ManagementError {
    ManagementError::unresolved(error.code, error.message)
}

/// Map a provider failure that happened on the paid decision boundary.
///
/// Almost every provider failure there may follow a request that was already on the wire, so it is
/// reported as an unresolved outcome. `ProviderNotStarted` is the exception the transport itself
/// proves: the configured provider executable was never spawned, so nothing was transmitted and no
/// inference can be outstanding. Reporting that as unresolved would strand the run in operator
/// reconciliation for a refusal that nothing needs to be reconciled with.
pub(crate) fn decision_provider_error(error: crate::episode::PolicyError) -> ManagementError {
    if let crate::episode::PolicyError::ProviderNotStarted = error {
        return ManagementError::unavailable(
            "provider_decision_not_started",
            "the provider transport never started, so no inference was requested",
        );
    }
    exchange_unresolved(provider_error(error))
}
