// SPDX-License-Identifier: MIT

//! Run-scoped process-lifecycle routes.
//!
//! Split from `http_routes_run.rs` so the run route table stays within the
//! repository file-size budget. The surface is deliberately thin: one
//! capability read, one command submission, and one identity-addressed
//! reconciliation read. Identity, instance, and authority all come from the
//! admitted run and the harness configuration, never from the request body.

use serde_json::Value;

use super::super::lifecycle::LifecycleCommand;
use super::routes::{decode_body_management, json_value};
use super::*;

/// Dispatches one lifecycle route, or `None` when the path is not lifecycle.
pub(super) fn dispatch_lifecycle_route(
    request: &HttpRequest,
    service: &ManagementService,
    actor: &super::super::auth::AuthContext,
    run_id: &str,
    segments: &[&str],
) -> Option<Result<Value, ManagementError>> {
    if !request.path.contains("/process-lifecycle") {
        return None;
    }
    Some(match (request.method.as_str(), segments) {
        ("GET", ["", "v1", "workflow-runs", _, "process-lifecycle"])
            if request.query.is_empty() =>
        {
            service
                .lifecycle_capability(actor, run_id)
                .and_then(|value| json_value(&value))
        }
        (
            "POST",
            [
                "",
                "v1",
                "workflow-runs",
                _,
                "process-lifecycle",
                "operations",
            ],
        ) if request.query.is_empty() => decode_body_management::<LifecycleCommand>(&request.body)
            .and_then(|command| {
                service
                    .lifecycle_command(actor, command)
                    .and_then(|value| json_value(&value))
            }),
        (
            "POST",
            [
                "",
                "v1",
                "workflow-runs",
                _,
                "process-lifecycle",
                "operations",
                operation_id,
                "reconcile",
            ],
        ) if request.query.is_empty() => {
            // Reconciliation is keyed entirely by the operation identity in the
            // path and the admitted run, so the request body carries nothing:
            // no instance, epoch, action, or caller-supplied answer is read.
            parse_operation_id(operation_id).and_then(|operation_id| {
                service
                    .reconcile_lifecycle_operation(actor, run_id, operation_id)
                    .and_then(|value| json_value(&value))
            })
        }
        _ => Err(ManagementError::invalid(
            "route_not_found",
            "management route was not found",
        )),
    })
}

/// Bounded decimal operation identity, rejecting signs and leading-zero aliasing.
fn parse_operation_id(value: &str) -> Result<u64, ManagementError> {
    if value.is_empty()
        || value.len() > 20
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || (value != "0" && value.starts_with('0'))
    {
        return Err(ManagementError::invalid(
            "lifecycle_operation_identity_invalid",
            "lifecycle operation identity is not a bounded decimal id",
        ));
    }
    value.parse::<u64>().map_err(|_| {
        ManagementError::invalid(
            "lifecycle_operation_identity_invalid",
            "lifecycle operation identity is not a bounded decimal id",
        )
    })
}
