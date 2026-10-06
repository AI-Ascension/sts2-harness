// SPDX-License-Identifier: MIT

use serde::Deserialize;
use serde_json::Value;

use super::super::context_owner::{
    CONTEXT_OWNER_PUBLICATION_LOOKUP_SCHEMA_VERSION, ContextOwnerDraftPublicationLookupRequest,
    ContextOwnerDraftPublicationRequest,
};
use super::routes::{decode_body_management, json_value};
use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PublishBody(ContextOwnerDraftPublicationRequest);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LookupBody(ContextOwnerDraftPublicationLookupRequest);

pub(super) fn dispatch(
    request: &HttpRequest,
    service: &ManagementService,
    actor: &super::super::auth::AuthContext,
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
                "context-owner-published-sources",
            ],
        ) if request.query.is_empty() => service
            .context_owner_published_sources(actor, run_id)
            .and_then(|value| json_value(&value)),
        (
            "POST",
            [
                "",
                "v1",
                "workflow-runs",
                _,
                "context-owner-drafts",
                draft_id,
                "publications",
            ],
        ) if request.query.is_empty() => {
            if let Err(error) =
                super::super::contract::validate_identifier("context_draft_id", draft_id)
            {
                return Some(Err(error.into()));
            }
            let body = match decode_body_management::<PublishBody>(&request.body) {
                Ok(body) => body.0,
                Err(error) => return Some(Err(error)),
            };
            service
                .publish_context_owner_draft(actor, run_id, draft_id, &body)
                .and_then(|value| json_value(&value))
        }
        (
            "POST",
            [
                "",
                "v1",
                "workflow-runs",
                _,
                "context-owner-draft-publication-receipts",
                "lookup",
            ],
        ) if request.query.is_empty() => {
            let body = match decode_body_management::<LookupBody>(&request.body) {
                Ok(body) => body.0,
                Err(error) => return Some(Err(error)),
            };
            if body.schema_version != CONTEXT_OWNER_PUBLICATION_LOOKUP_SCHEMA_VERSION {
                return Some(Err(ManagementError::invalid(
                    "context_publication_lookup_schema",
                    "unsupported publication receipt lookup schema",
                )));
            }
            service
                .recover_context_owner_publication_receipt(actor, run_id, &body)
                .and_then(|value| json_value(&value))
        }
        _ => return None,
    };
    Some(result)
}
