// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use serde_json::Value;

use super::super::contract::{
    PROVIDER_SESSION_POLICY_COMMAND_SCHEMA_VERSION, ProviderSessionPolicyAdoptImportedRequest,
    ProviderSessionPolicyApprovalRequest, validate_digest, validate_identifier,
};
use super::super::provider_policy::ProviderSessionPolicyOwnerCommand;
use super::super::{AuthContext, ManagementError, ManagementService};
use super::HttpRequest;
use super::routes::{decode_body_management, json_value};

pub(super) fn dispatch_provider_policy_route(
    request: &HttpRequest,
    service: &ManagementService,
    actor: &AuthContext,
    run_id: &str,
    segments: &[&str],
) -> Option<Result<Value, ManagementError>> {
    let result = match (request.method.as_str(), segments) {
        ("GET", ["", "v1", "workflow-runs", _, "provider-session-policy"])
            if request.query.is_empty() =>
        {
            service
                .provider_session_policy(actor, run_id)
                .and_then(|value| json_value(&value))
        }
        (
            "POST",
            [
                "",
                "v1",
                "workflow-runs",
                _,
                "provider-session-policy",
                "import",
            ],
        ) => {
            let result = policy_revision_query(&request.query).and_then(|expected_revision| {
                service.provider_session_policy_command(
                    actor,
                    run_id,
                    ProviderSessionPolicyOwnerCommand::Import {
                        policy_bytes: request.body.clone(),
                        expected_revision,
                    },
                )
            });
            result.and_then(|value| json_value(&value))
        }
        (
            "POST",
            [
                "",
                "v1",
                "workflow-runs",
                _,
                "provider-session-policy",
                "proposals",
                proposal_id,
            ],
        ) => {
            let result = policy_proposal_query(&request.query).and_then(
                |(source_sha256, expected_revision)| {
                    validate_identifier("proposal_id", proposal_id)?;
                    service.provider_session_policy_command(
                        actor,
                        run_id,
                        ProviderSessionPolicyOwnerCommand::Propose {
                            proposal_id: (*proposal_id).to_owned(),
                            source_sha256,
                            target_policy_bytes: request.body.clone(),
                            expected_revision,
                        },
                    )
                },
            );
            result.and_then(|value| json_value(&value))
        }
        (
            "POST",
            [
                "",
                "v1",
                "workflow-runs",
                _,
                "provider-session-policy",
                "proposals",
                proposal_id,
                "approve",
            ],
        ) => {
            let result = policy_revision_query(&request.query).and_then(|expected_revision| {
                validate_identifier("proposal_id", proposal_id)?;
                let body: ProviderSessionPolicyApprovalRequest =
                    decode_body_management(&request.body)?;
                if body.schema_version != PROVIDER_SESSION_POLICY_COMMAND_SCHEMA_VERSION {
                    return Err(ManagementError::invalid(
                        "schema_version_mismatch",
                        "provider-session policy command schema version is not supported",
                    ));
                }
                validate_digest("proposal_sha256", &body.proposal_sha256)?;
                validate_identifier("approval_ref", &body.approval_ref)?;
                service.provider_session_policy_command(
                    actor,
                    run_id,
                    ProviderSessionPolicyOwnerCommand::Approve {
                        proposal_id: (*proposal_id).to_owned(),
                        proposal_sha256: body.proposal_sha256,
                        approval_ref: body.approval_ref,
                        expected_revision,
                    },
                )
            });
            result.and_then(|value| json_value(&value))
        }
        (
            "POST",
            [
                "",
                "v1",
                "workflow-runs",
                _,
                "provider-session-policy",
                "proposals",
                proposal_id,
                "adopt",
            ],
        ) => {
            let result = policy_revision_query(&request.query).and_then(|expected_revision| {
                validate_identifier("proposal_id", proposal_id)?;
                let body: ProviderSessionPolicyApprovalRequest =
                    decode_body_management(&request.body)?;
                if body.schema_version != PROVIDER_SESSION_POLICY_COMMAND_SCHEMA_VERSION {
                    return Err(ManagementError::invalid(
                        "schema_version_mismatch",
                        "provider-session policy command schema version is not supported",
                    ));
                }
                validate_digest("proposal_sha256", &body.proposal_sha256)?;
                validate_identifier("approval_ref", &body.approval_ref)?;
                service.provider_session_policy_command(
                    actor,
                    run_id,
                    ProviderSessionPolicyOwnerCommand::Adopt {
                        proposal_id: (*proposal_id).to_owned(),
                        proposal_sha256: body.proposal_sha256,
                        approval_ref: body.approval_ref,
                        expected_revision,
                    },
                )
            });
            result.and_then(|value| json_value(&value))
        }
        (
            "POST",
            [
                "",
                "v1",
                "workflow-runs",
                _,
                "provider-session-policy",
                "adoptions",
            ],
        ) => {
            let result = policy_revision_query(&request.query).and_then(|expected_revision| {
                let body: ProviderSessionPolicyAdoptImportedRequest =
                    decode_body_management(&request.body)?;
                if body.schema_version != PROVIDER_SESSION_POLICY_COMMAND_SCHEMA_VERSION {
                    return Err(ManagementError::invalid(
                        "schema_version_mismatch",
                        "provider-session policy command schema version is not supported",
                    ));
                }
                validate_digest("policy_sha256", &body.policy_sha256)?;
                service.provider_session_policy_command(
                    actor,
                    run_id,
                    ProviderSessionPolicyOwnerCommand::AdoptImported {
                        policy_sha256: body.policy_sha256,
                        expected_revision,
                    },
                )
            });
            result.and_then(|value| json_value(&value))
        }
        _ => return None,
    };
    Some(result)
}

fn policy_revision_query(query: &BTreeMap<String, String>) -> Result<u64, ManagementError> {
    if query.len() != 1 || !query.contains_key("expected_revision") {
        return Err(ManagementError::invalid(
            "invalid_query",
            "provider-session policy commands require only expected_revision",
        ));
    }
    let revision = query["expected_revision"].parse::<u64>().map_err(|_| {
        ManagementError::invalid("invalid_query", "expected_revision is not an integer")
    })?;
    if revision == 0 {
        return Err(ManagementError::invalid(
            "invalid_query",
            "expected_revision must be positive",
        ));
    }
    Ok(revision)
}

fn policy_proposal_query(
    query: &BTreeMap<String, String>,
) -> Result<(String, u64), ManagementError> {
    if query.len() != 2 || !query.contains_key("expected_revision") {
        return Err(ManagementError::invalid(
            "invalid_query",
            "provider-session policy proposal requires source_sha256 and expected_revision",
        ));
    }
    let source_sha256 = query.get("source_sha256").ok_or_else(|| {
        ManagementError::invalid(
            "invalid_query",
            "provider-session policy proposal source_sha256 is missing",
        )
    })?;
    validate_digest("source_sha256", source_sha256)?;
    let revision = query["expected_revision"].parse::<u64>().map_err(|_| {
        ManagementError::invalid("invalid_query", "expected_revision is not an integer")
    })?;
    if revision == 0 {
        return Err(ManagementError::invalid(
            "invalid_query",
            "expected_revision must be positive",
        ));
    }
    Ok((source_sha256.clone(), revision))
}
