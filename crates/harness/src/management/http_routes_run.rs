// SPDX-License-Identifier: MIT

//! Run-scoped management routes. Split out of `http_routes` so the route table
//! stays within the repository file-size budget while run routes keep their
//! exact match patterns.

use std::collections::BTreeMap;

use serde_json::Value;

use super::super::context_owner::ContextControlCommand;
use super::routes::{decode_body_management, json_value};
use super::*;

pub(super) fn dispatch_run_route(
    request: &HttpRequest,
    service: &ManagementService,
    actor: &super::super::auth::AuthContext,
) -> Result<Value, ManagementError> {
    let segments = request.path.split('/').collect::<Vec<_>>();
    if segments.len() < 4 || segments[1] != "v1" || segments[2] != "workflow-runs" {
        return Err(ManagementError::invalid(
            "route_not_found",
            "management route was not found",
        ));
    }
    let run_id = segments[3];
    validate_identifier("run_id", run_id).map_err(ManagementError::from)?;
    match (request.method.as_str(), segments.as_slice()) {
        ("GET", ["", "v1", "workflow-runs", _]) if request.query.is_empty() => service
            .status(actor, run_id)
            .and_then(|value| json_value(&value)),
        ("GET", ["", "v1", "workflow-runs", _, "context-owner-association"])
            if request.query.is_empty() =>
        {
            service
                .current_context_owner_association(actor, run_id)
                .and_then(|value| json_value(&value))
        }
        (
            "GET",
            [
                "",
                "v1",
                "workflow-runs",
                _,
                "context-owner-effective-limits",
            ],
        ) if request.query.is_empty() => service
            .current_context_effective_limits(actor, run_id)
            .and_then(|value| json_value(&value)),
        ("GET", ["", "v1", "workflow-runs", _, "context"]) if request.query.is_empty() => service
            .context_association(actor, run_id)
            .and_then(|value| json_value(&value)),
        (
            "GET",
            [
                "",
                "v1",
                "workflow-runs",
                _,
                "executions",
                _,
                "context-binding",
            ],
        ) if request.query.is_empty() => service
            .recorded_context_binding_projection(actor, run_id, segments[5])
            .and_then(|value| json_value(&value)),
        (
            "POST",
            [
                "",
                "v1",
                "workflow-runs",
                _,
                "context-control-receipts",
                "lookup",
            ],
        ) if request.query.is_empty() => {
            let command: ContextControlCommand = decode_body_management(&request.body)?;
            service
                .recover_context_control_receipt(actor, run_id, &command)
                .and_then(|value| json_value(&value))
        }
        ("GET", ["", "v1", "workflow-runs", _, "provider-sessions"])
            if request.query.is_empty() =>
        {
            service
                .provider_sessions(actor, run_id)
                .and_then(|value| json_value(&value))
        }
        ("GET", ["", "v1", "workflow-runs", _, "events"]) => {
            let after = query_u64(&request.query, "after_sequence", 0)?;
            let limit = query_u64(
                &request.query,
                "limit",
                super::super::contract::MAX_EVENTS_PER_PAGE,
            )?;
            service
                .events(actor, run_id, after, limit)
                .and_then(|value| json_value(&value))
        }
        ("POST", ["", "v1", "workflow-runs", _, "commands"]) if request.query.is_empty() => {
            let body: CommandRequest = decode_body_management(&request.body)?;
            if body.run_id != run_id {
                return Err(ManagementError::invalid(
                    "run_id_mismatch",
                    "command run_id does not match the path",
                ));
            }
            service
                .command(actor, body)
                .and_then(|value| json_value(&value))
        }
        ("POST", ["", "v1", "workflow-runs", _, "replay"]) if request.query.is_empty() => {
            let body: ReplayRequest = decode_body_management(&request.body)?;
            if body.run_id != run_id {
                return Err(ManagementError::invalid(
                    "run_id_mismatch",
                    "replay run_id does not match the path",
                ));
            }
            service
                .replay(actor, body)
                .and_then(|value| json_value(&value))
        }
        ("POST", ["", "v1", "workflow-runs", _, "export"]) if request.query.is_empty() => {
            let body: ExportRequest = decode_body_management(&request.body)?;
            if body.run_id != run_id {
                return Err(ManagementError::invalid(
                    "run_id_mismatch",
                    "export run_id does not match the path",
                ));
            }
            service
                .export(actor, body)
                .and_then(|value| json_value(&value))
        }
        ("GET", ["", "v1", "workflow-runs", _, "artifacts", _]) if request.query.is_empty() => {
            Err(ManagementError::unavailable(
                "artifact_port_unavailable",
                "artifact retrieval is not injected into the management adapter",
            ))
        }
        _ => Err(ManagementError::invalid(
            "route_not_found",
            "management route was not found",
        )),
    }
}

pub(super) fn query_u64(
    query: &BTreeMap<String, String>,
    name: &str,
    default: u64,
) -> Result<u64, ManagementError> {
    for key in query.keys() {
        if !matches!(key.as_str(), "after_sequence" | "limit") {
            return Err(ManagementError::invalid(
                "unknown_query",
                "query parameter is not supported",
            ));
        }
    }
    query
        .get(name)
        .map(|value| {
            value.parse::<u64>().map_err(|_| {
                ManagementError::invalid("invalid_query", "query parameter is not an integer")
            })
        })
        .transpose()
        .map(|value| value.unwrap_or(default))
}
