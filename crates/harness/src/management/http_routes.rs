// SPDX-License-Identifier: MIT

use serde::de::DeserializeOwned;
use serde_json::Value;

use super::super::context_owner::ContextBindingRequest;
use super::super::contract_authoring::{
    StudioCreateDraftRequest, StudioPublishDraftRequest, StudioSaveDraftRequest,
};
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
    let bearer = parse_bearer(request.headers.get("authorization"))?;
    let actor = authenticator
        .authenticate(bearer)
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
            ("GET", "/v1/workflow-targets") if request.query.is_empty() => service
                .target_catalog(&actor)
                .and_then(|value| json_value(&value)),
            ("POST", "/v1/workflow-targets/preflight") if request.query.is_empty() => {
                let body: TargetAdmissionRequest = decode_body_management(&request.body)?;
                service
                    .preflight_target(&actor, body)
                    .and_then(|value| json_value(&value))
            }
            ("GET", "/v1/context-bindings") if request.query.is_empty() => service
                .context_owner_catalog(&actor)
                .and_then(|value| json_value(&value)),
            ("POST", "/v1/context-bindings/bind") if request.query.is_empty() => {
                let body: ContextBindingRequest = decode_body_management(&request.body)?;
                service
                    .bind_context(&actor, body)
                    .and_then(|value| json_value(&value))
            }
            ("GET", "/v1/inference-profiles") if request.query.is_empty() => service
                .inference_profile_catalog(&actor)
                .and_then(|value| json_value(&value)),
            ("GET", "/v1/capabilities") if request.query.is_empty() => service
                .capabilities(&actor)
                .and_then(|value| json_value(&value)),
            ("GET", "/v1/studio/definitions") if request.query.is_empty() => service
                .studio_definitions(&actor)
                .and_then(|value| json_value(&value)),
            ("POST", "/v1/studio/drafts") if request.query.is_empty() => {
                let body: StudioCreateDraftRequest = decode_body_management(&request.body)?;
                service
                    .studio_create_draft(&actor, body)
                    .and_then(|value| json_value(&value))
            }
            _ => dispatch_studio_or_run_route(&request, service, &actor, bearer),
        })();
    match result {
        Ok(body) => json_response(200, body),
        Err(error) => Err(HttpError::from_management(error)),
    }
}

fn dispatch_studio_or_run_route(
    request: &HttpRequest,
    service: &ManagementService,
    actor: &super::super::auth::AuthContext,
    bearer: Option<&str>,
) -> Result<Value, ManagementError> {
    if request.path.starts_with("/v1/studio/") {
        return dispatch_studio_route(request, service, actor);
    }
    if request.path.starts_with("/v1/memory-policy-owner") {
        return super::routes_memory_owner::dispatch_memory_policy_owner_route(
            request, service, actor, bearer,
        )
        .unwrap_or_else(|| {
            Err(ManagementError::invalid(
                "route_not_found",
                "management route was not found",
            ))
        });
    }
    super::routes_run::dispatch_run_route(request, service, actor)
}

fn dispatch_studio_route(
    request: &HttpRequest,
    service: &ManagementService,
    actor: &super::super::auth::AuthContext,
) -> Result<Value, ManagementError> {
    let segments = request.path.split('/').collect::<Vec<_>>();
    if segments.len() < 5
        || segments[1] != "v1"
        || segments[2] != "studio"
        || segments[3] != "drafts"
    {
        return Err(ManagementError::invalid(
            "route_not_found",
            "management route was not found",
        ));
    }
    let draft_id = segments[4];
    validate_identifier("draft_id", draft_id).map_err(ManagementError::from)?;
    match (request.method.as_str(), segments.as_slice()) {
        ("GET", ["", "v1", "studio", "drafts", _]) if request.query.is_empty() => service
            .studio_draft(actor, draft_id)
            .and_then(|value| json_value(&value)),
        ("PUT", ["", "v1", "studio", "drafts", _]) if request.query.is_empty() => {
            let body: StudioSaveDraftRequest = decode_body_management(&request.body)?;
            service
                .studio_save_draft(actor, draft_id, body)
                .and_then(|value| json_value(&value))
        }
        ("POST", ["", "v1", "studio", "drafts", _, "publish"]) if request.query.is_empty() => {
            let body: StudioPublishDraftRequest = decode_body_management(&request.body)?;
            service
                .studio_publish_draft(actor, draft_id, body)
                .and_then(|value| json_value(&value))
        }
        _ => Err(ManagementError::invalid(
            "route_not_found",
            "management route was not found",
        )),
    }
}

pub(super) fn decode_body_management<T: DeserializeOwned>(
    body: &[u8],
) -> Result<T, ManagementError> {
    decode_strict(body).map_err(ManagementError::from)
}

pub(super) fn json_value<T: serde::Serialize>(value: &T) -> Result<Value, ManagementError> {
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
