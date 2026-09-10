// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde_json::Value;

use super::response::reason_phrase;
use super::*;

pub(super) fn dispatch(
    request: HttpRequest,
    service: &ManagementService,
    authenticator: &dyn Authenticator,
) -> Result<HttpResponse, HttpError> {
    if request.method == "GET" && request.path == "/v1/health" {
        return json_response(200, service.health());
    }
    let actor = authenticator
        .authenticate(parse_bearer(request.headers.get("authorization"))?)
        .map_err(auth_http_error)?;
    let result: Result<Value, ManagementError> =
        (|| match (request.method.as_str(), request.path.as_str()) {
            ("POST", "/v1/workflow-definitions/validate") => {
                let body: ValidateRequest = decode_body_management(&request.body)?;
                service
                    .validate(&actor, body)
                    .and_then(|value| json_value(&value))
            }
            ("POST", "/v1/workflow-definitions/inspect") => {
                let body: InspectRequest = decode_body_management(&request.body)?;
                service
                    .inspect(&actor, body)
                    .and_then(|value| json_value(&value))
            }
            ("POST", "/v1/workflow-definitions/diff") => {
                let body: DiffRequest = decode_body_management(&request.body)?;
                service
                    .diff(&actor, body)
                    .and_then(|value| json_value(&value))
            }
            ("POST", "/v1/workflow-runs") => {
                let body: RunRequest = decode_body_management(&request.body)?;
                service
                    .submit_run(&actor, body)
                    .and_then(|value| json_value(&value))
            }
            ("GET", "/v1/capabilities") if request.query.is_empty() => service
                .capabilities(&actor)
                .and_then(|value| json_value(&value)),
            _ => dispatch_run_route(&request, service, &actor),
        })();
    match result {
        Ok(body) => json_response(200, body),
        Err(error) => Err(HttpError::from_management(error)),
    }
}

fn dispatch_run_route(
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

fn query_u64(
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

fn decode_body_management<T: DeserializeOwned>(body: &[u8]) -> Result<T, ManagementError> {
    decode_strict(body).map_err(ManagementError::from)
}

fn json_value<T: serde::Serialize>(value: &T) -> Result<Value, ManagementError> {
    serde_json::to_value(value)
        .map_err(|error| ManagementError::store("response_encode", error.to_string()))
}

fn json_response<T: serde::Serialize>(status: u16, value: T) -> Result<HttpResponse, HttpError> {
    let body = serde_json::to_vec(&value)
        .map_err(|error| HttpError::new("response_encode", error.to_string()))?;
    if body.len() > MAX_RESPONSE_BYTES {
        return Err(HttpError::new(
            "response_too_large",
            "response exceeds the bound",
        ));
    }
    Ok(HttpResponse {
        status,
        reason: reason_phrase(status),
        body,
    })
}
