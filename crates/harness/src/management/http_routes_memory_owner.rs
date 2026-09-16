// SPDX-License-Identifier: MIT

//! Loopback-only, authenticated management routes for the selected-memory
//! policy owner used by a Runtime-v3 process before game/provider admission.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

use super::super::contract::{validate_digest, validate_identifier};
use super::super::{AuthContext, ManagementError, ManagementService};
use super::HttpRequest;
use super::routes::{decode_body_management, json_value};
use crate::context_memory::policy_owner::{PolicyCommand, SavedPolicyRef};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewDigest {
    review_sha256: String,
}

pub(super) fn dispatch_memory_policy_owner_route(
    request: &HttpRequest,
    service: &ManagementService,
    actor: &AuthContext,
    bearer: Option<&str>,
) -> Option<Result<Value, ManagementError>> {
    let segments = request.path.split('/').collect::<Vec<_>>();
    let result = match (request.method.as_str(), segments.as_slice()) {
        ("GET", ["", "v1", "memory-policy-owner"]) if request.query.is_empty() => {
            service.memory_policy_owner_current(actor, bearer)
        }
        (
            "GET",
            [
                "",
                "v1",
                "memory-policy-owner",
                "policies",
                policy_id,
                version,
                raw_sha256,
            ],
        ) if request.query.is_empty() => {
            let reference = (|| {
                validate_identifier("policy_id", policy_id)?;
                validate_digest("raw_sha256", raw_sha256)?;
                let version = version.parse::<u64>().map_err(|_| {
                    ManagementError::invalid("invalid_policy_version", "policy version is invalid")
                })?;
                if version == 0 {
                    return Err(ManagementError::invalid(
                        "invalid_policy_version",
                        "policy version must be positive",
                    ));
                }
                Ok(SavedPolicyRef {
                    policy_id: (*policy_id).to_owned(),
                    version,
                    raw_sha256: (*raw_sha256).to_owned(),
                })
            })();
            reference.and_then(|reference| {
                service
                    .memory_policy_owner_inspect_policy(actor, bearer, &reference)
                    .map(|raw_hex| {
                        serde_json::json!({
                            "schema_version": "ascension.memory-policy-owner.policy-content.v1",
                            "reference": reference,
                            "raw_hex": raw_hex,
                        })
                    })
            })
        }
        ("GET", ["", "v1", "memory-policy-owner", "reviews", review_id])
            if request.query.is_empty() =>
        {
            validate_identifier("review_id", review_id)
                .map_err(ManagementError::from)
                .and_then(|()| service.memory_policy_owner_inspect_review(actor, bearer, review_id))
        }
        ("POST", ["", "v1", "memory-policy-owner", "proposals", review_id]) => {
            proposal_query(&request.query).and_then(|(source, expected_active_version)| {
                validate_identifier("review_id", review_id).map_err(ManagementError::from)?;
                let key = idempotency_key(request)?;
                service.memory_policy_owner_execute(
                    actor,
                    bearer,
                    PolicyCommand::ProposeRevalidation {
                        key,
                        review_id: (*review_id).to_owned(),
                        source,
                        target_raw: request.body.clone(),
                        expected_active_version,
                    },
                )
            })
        }
        (
            "POST",
            [
                "",
                "v1",
                "memory-policy-owner",
                "proposals",
                review_id,
                "approve",
            ],
        ) => review_command(request, service, actor, bearer, review_id, true),
        (
            "POST",
            [
                "",
                "v1",
                "memory-policy-owner",
                "proposals",
                review_id,
                "adopt",
            ],
        ) => review_command(request, service, actor, bearer, review_id, false),
        _ => return None,
    };
    Some(result.and_then(|value| {
        if value.is_object() {
            Ok(value)
        } else {
            json_value(&value)
        }
    }))
}

fn review_command(
    request: &HttpRequest,
    service: &ManagementService,
    actor: &AuthContext,
    bearer: Option<&str>,
    review_id: &str,
    approve: bool,
) -> Result<Value, ManagementError> {
    if !request.query.is_empty() {
        return Err(ManagementError::invalid(
            "invalid_query",
            "policy review commands do not accept query parameters",
        ));
    }
    validate_identifier("review_id", review_id).map_err(ManagementError::from)?;
    let body: ReviewDigest = decode_body_management(&request.body)?;
    validate_digest("review_sha256", &body.review_sha256)?;
    let key = idempotency_key(request)?;
    let command = if approve {
        PolicyCommand::Approve {
            key,
            review_id: review_id.to_owned(),
            review_sha256: body.review_sha256,
        }
    } else {
        PolicyCommand::Adopt {
            key,
            review_id: review_id.to_owned(),
            review_sha256: body.review_sha256,
        }
    };
    service.memory_policy_owner_execute(actor, bearer, command)
}

fn proposal_query(
    query: &BTreeMap<String, String>,
) -> Result<(SavedPolicyRef, u64), ManagementError> {
    const REQUIRED: [&str; 4] = [
        "source_policy_id",
        "source_version",
        "source_raw_sha256",
        "expected_active_version",
    ];
    if query.len() != REQUIRED.len() || REQUIRED.iter().any(|key| !query.contains_key(*key)) {
        return Err(ManagementError::invalid(
            "invalid_query",
            "revalidation requires the source reference and expected active version",
        ));
    }
    let source_policy_id = &query["source_policy_id"];
    validate_identifier("source_policy_id", source_policy_id).map_err(ManagementError::from)?;
    let source_version = positive_query_u64(query, "source_version")?;
    let expected_active_version = positive_query_u64(query, "expected_active_version")?;
    let source_raw_sha256 = &query["source_raw_sha256"];
    validate_digest("source_raw_sha256", source_raw_sha256)?;
    Ok((
        SavedPolicyRef {
            policy_id: source_policy_id.clone(),
            version: source_version,
            raw_sha256: source_raw_sha256.clone(),
        },
        expected_active_version,
    ))
}

fn positive_query_u64(
    query: &BTreeMap<String, String>,
    name: &str,
) -> Result<u64, ManagementError> {
    let value = query[name].parse::<u64>().map_err(|_| {
        ManagementError::invalid("invalid_query", "policy version is not an integer")
    })?;
    if value == 0 {
        return Err(ManagementError::invalid(
            "invalid_query",
            "policy version must be positive",
        ));
    }
    Ok(value)
}

fn idempotency_key(request: &HttpRequest) -> Result<String, ManagementError> {
    let key = request
        .headers
        .get("idempotency-key")
        .ok_or_else(|| {
            ManagementError::invalid(
                "idempotency_key_required",
                "policy mutations require an Idempotency-Key header",
            )
        })?
        .as_str();
    if key.is_empty()
        || key.len() > 128
        || !key.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && b"._:-".contains(&byte))
        })
    {
        return Err(ManagementError::invalid(
            "invalid_idempotency_key",
            "policy idempotency key is invalid",
        ));
    }
    Ok(key.to_owned())
}
