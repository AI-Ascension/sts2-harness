// SPDX-License-Identifier: MIT

//! Additive run-scoped routes for owner-managed drafts and previews.

use serde::Deserialize;
use serde_json::Value;

use super::super::context_owner::{
    ContextOwnerDraftCreateRequest, ContextOwnerDraftPatchRequest,
    ContextOwnerMutationLookupRequest, ContextOwnerPreviewRequest,
    CONTEXT_OWNER_MUTATION_LOOKUP_SCHEMA_VERSION,
};
use super::super::contract::validate_identifier;
use super::routes::{decode_body_management, json_value};
use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftCreateBody(ContextOwnerDraftCreateRequest);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftPatchBody(ContextOwnerDraftPatchRequest);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreviewBody(ContextOwnerPreviewRequest);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MutationLookupBody(ContextOwnerMutationLookupRequest);

pub(super) fn dispatch(
    request: &HttpRequest,
    service: &ManagementService,
    actor: &super::super::auth::AuthContext,
    run_id: &str,
    segments: &[&str],
) -> Option<Result<Value, ManagementError>> {
    let result = match (request.method.as_str(), segments) {
        ("GET", ["", "v1", "workflow-runs", _, "context-owner-items"]) => {
            let (draft_id, include_content) = match context_items_query(&request.query) {
                Ok(value) => value,
                Err(error) => return Some(Err(error)),
            };
            service
                .eligible_context_owner_items(actor, run_id, draft_id.as_deref(), include_content)
                .and_then(|value| json_value(&value))
        }
        ("GET", ["", "v1", "workflow-runs", _, "context-owner-drafts"])
            if request.query.is_empty() =>
        {
            service
                .context_owner_drafts(actor, run_id)
                .and_then(|value| json_value(&value))
        }
        ("POST", ["", "v1", "workflow-runs", _, "context-owner-drafts"])
            if request.query.is_empty() =>
        {
            let body = match decode_body_management::<DraftCreateBody>(&request.body) {
                Ok(body) => body.0,
                Err(error) => return Some(Err(error)),
            };
            service
                .create_context_owner_draft(actor, run_id, &body)
                .and_then(|value| json_value(&value))
        }
        (
            "GET",
            [
                "",
                "v1",
                "workflow-runs",
                _,
                "context-owner-drafts",
                draft_id,
            ],
        ) if request.query.is_empty() => {
            if let Err(error) = validate_identifier("context_draft_id", draft_id) {
                return Some(Err(error.into()));
            }
            service
                .context_owner_draft(actor, run_id, draft_id)
                .and_then(|value| json_value(&value))
        }
        (
            "PATCH",
            [
                "",
                "v1",
                "workflow-runs",
                _,
                "context-owner-drafts",
                draft_id,
            ],
        ) if request.query.is_empty() => {
            if let Err(error) = validate_identifier("context_draft_id", draft_id) {
                return Some(Err(error.into()));
            }
            let body = match decode_body_management::<DraftPatchBody>(&request.body) {
                Ok(body) if body.0.draft_id == *draft_id => body.0,
                Ok(_) => {
                    return Some(Err(ManagementError::invalid(
                        "context_draft_path_mismatch",
                        "draft ID in the path must match the request envelope",
                    )));
                }
                Err(error) => return Some(Err(error)),
            };
            service
                .patch_context_owner_draft(actor, run_id, &body)
                .and_then(|value| json_value(&value))
        }
        (
            "POST",
            [
                "",
                "v1",
                "workflow-runs",
                _,
                "context-owner-drafts",
                draft_id,
                "previews",
            ],
        ) if request.query.is_empty() => {
            if let Err(error) = validate_identifier("context_draft_id", draft_id) {
                return Some(Err(error.into()));
            }
            let body = match decode_body_management::<PreviewBody>(&request.body) {
                Ok(body) if body.0.draft_id == *draft_id => body.0,
                Ok(_) => {
                    return Some(Err(ManagementError::invalid(
                        "context_draft_path_mismatch",
                        "draft ID in the path must match the request envelope",
                    )));
                }
                Err(error) => return Some(Err(error)),
            };
            service
                .create_context_owner_preview(actor, run_id, &body)
                .and_then(|value| json_value(&value))
        }
        ("GET", ["", "v1", "workflow-runs", _, "context-owner-revisions"]) => {
            let (after, limit) = match revision_query(&request.query) {
                Ok(query) => query,
                Err(error) => return Some(Err(error)),
            };
            service
                .context_owner_revisions(actor, run_id, after.as_deref(), limit)
                .and_then(|value| json_value(&value))
        }
        (
            "GET",
            [
                "",
                "v1",
                "workflow-runs",
                _,
                "context-owner-revisions",
                revision_id,
            ],
        ) if request.query.is_empty() => {
            if let Err(error) = validate_identifier("context_revision_id", revision_id) {
                return Some(Err(error.into()));
            }
            service
                .context_owner_revision(actor, run_id, revision_id)
                .and_then(|value| json_value(&value))
        }
        (
            "GET",
            [
                "",
                "v1",
                "workflow-runs",
                _,
                "context-owner-previews",
                preview_id,
            ],
        ) if request.query.is_empty() => {
            if let Err(error) = validate_identifier("context_preview_id", preview_id) {
                return Some(Err(error.into()));
            }
            service
                .context_owner_preview(actor, run_id, preview_id)
                .and_then(|value| json_value(&value))
        }
        (
            "POST",
            [
                "",
                "v1",
                "workflow-runs",
                _,
                "context-owner-mutation-receipts",
                "lookup",
            ],
        ) if request.query.is_empty() => {
            let body = match decode_body_management::<MutationLookupBody>(&request.body) {
                Ok(body) => body.0,
                Err(error) => return Some(Err(error)),
            };
            if body.schema_version != CONTEXT_OWNER_MUTATION_LOOKUP_SCHEMA_VERSION {
                return Some(Err(ManagementError::invalid(
                    "context_mutation_lookup_schema",
                    "unsupported owner mutation lookup schema",
                )));
            }
            service
                .recover_context_owner_mutation_receipt(actor, run_id, &body.request)
                .and_then(|value| json_value(&value))
        }
        _ => return None,
    };
    Some(result)
}

fn context_items_query(
    query: &std::collections::BTreeMap<String, String>,
) -> Result<(Option<String>, bool), ManagementError> {
    if query
        .keys()
        .any(|key| !matches!(key.as_str(), "draft_id" | "include_content"))
    {
        return Err(ManagementError::invalid(
            "unknown_query",
            "context item query parameter is not supported",
        ));
    }
    let draft_id = query.get("draft_id").cloned();
    if let Some(value) = draft_id.as_deref() {
        validate_identifier("context_draft_id", value)?;
    }
    let include_content = match query.get("include_content").map(String::as_str) {
        None | Some("false") => false,
        Some("true") => true,
        Some(_) => {
            return Err(ManagementError::invalid(
                "invalid_query",
                "include_content must be true or false",
            ));
        }
    };
    Ok((draft_id, include_content))
}

fn revision_query(
    query: &std::collections::BTreeMap<String, String>,
) -> Result<(Option<String>, u64), ManagementError> {
    if query
        .keys()
        .any(|key| !matches!(key.as_str(), "after_revision_id" | "limit"))
    {
        return Err(ManagementError::invalid(
            "unknown_query",
            "revision query parameter is not supported",
        ));
    }
    let after = query.get("after_revision_id").cloned();
    let limit = query
        .get("limit")
        .map(|value| {
            value.parse::<u64>().map_err(|_| {
                ManagementError::invalid("invalid_query", "revision limit is not an integer")
            })
        })
        .transpose()?
        .unwrap_or(super::super::context_owner::MAX_CONTEXT_OWNER_PAGE_SIZE);
    Ok((after, limit))
}
