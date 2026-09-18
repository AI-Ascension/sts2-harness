// SPDX-License-Identifier: MIT

//! Run-scoped effective-limit record routes. Split out of `http_routes_run` so
//! the run route table stays within the repository file-size budget while these
//! routes keep their exact match patterns.

use serde_json::Value;

use super::super::{AuthContext, ManagementError, ManagementService};
use super::HttpRequest;
use super::routes::json_value;

pub(super) fn dispatch_effective_limits_route(
    request: &HttpRequest,
    service: &ManagementService,
    actor: &AuthContext,
    run_id: &str,
    segments: &[&str],
) -> Option<Result<Value, ManagementError>> {
    let result = match (request.method.as_str(), segments) {
        (
            "GET",
            [
                "",
                "v1",
                "workflow-runs",
                _,
                "provider-session-effective-limits",
            ],
        ) if request.query.is_empty() => service
            .provider_session_effective_limits(actor, run_id)
            .and_then(|value| json_value(&value)),
        (
            "GET",
            [
                "",
                "v1",
                "workflow-runs",
                _,
                "context-memory-effective-limits",
            ],
        ) if request.query.is_empty() => service
            .context_memory_effective_limits(actor, run_id)
            .and_then(|value| json_value(&value)),
        _ => return None,
    };
    Some(result)
}
