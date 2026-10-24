// SPDX-License-Identifier: MIT

use super::super::auth::AuthContext;
use super::super::contract::{DiagnosticSeverity, RunSnapshot, digest_value};
use super::super::contract_seed_v2::{
    SeedBindingReadbackV2, SeededRunSubmissionResponseV2, WORKFLOW_RUN_SUBMISSION_V2_SCHEMA,
    WorkflowRunRequestV2,
};
use super::super::seed_key::SeedKeyError;
use super::super::service::{
    DefinitionPort, ManagementError, ManagementService, run_submission_response,
};
use super::super::support::verify_digest;

pub(super) fn validate_definition(
    service: &ManagementService,
    request: &WorkflowRunRequestV2,
) -> Result<String, ManagementError> {
    if let Some(definition) = request.definition.as_ref() {
        let capabilities = match service.capabilities.capabilities() {
            Ok(value) => value,
            Err(error) if error.class == super::super::contract::ErrorClass::Unavailable => {
                serde_json::json!({ "capabilities": [] })
            }
            Err(error) => return Err(error),
        };
        let validation = service.definitions.validate(definition, &capabilities)?;
        if validation
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
        {
            return Err(ManagementError::invalid(
                "definition_invalid",
                "workflow definition validation returned an error diagnostic",
            ));
        }
        let digest = digest_value(definition)?;
        verify_digest(&digest, &validation.definition_digest, "definition")?;
        Ok(digest)
    } else {
        let artifact_id = request.artifact_id.as_deref().ok_or_else(|| {
            ManagementError::invalid("run_source_count", "workflow source is missing")
        })?;
        digest_value(&serde_json::json!({ "artifact_id": artifact_id }))
    }
}

pub(super) fn response(
    snapshot: &RunSnapshot,
    seed_binding: SeedBindingReadbackV2,
) -> SeededRunSubmissionResponseV2 {
    SeededRunSubmissionResponseV2 {
        schema_version: WORKFLOW_RUN_SUBMISSION_V2_SCHEMA.to_owned(),
        run: run_submission_response(snapshot),
        seed_binding,
    }
}

pub(super) fn seed_request_error(_error: crate::seed_binding::SeedBindingError) -> ManagementError {
    ManagementError::invalid("seed_request_invalid", "versioned seed request is invalid")
}

pub(super) fn seed_key_error(error: SeedKeyError) -> ManagementError {
    match error {
        SeedKeyError::InvalidIdentity => ManagementError::invalid(
            "seed_identity_invalid",
            "versioned seed identity or request is invalid",
        ),
        _ => ManagementError::unavailable(
            "seed_key_authority_unavailable",
            "the pinned seed derivation authority is unavailable or failed verification",
        ),
    }
}

pub(super) fn seed_binding_not_found() -> ManagementError {
    ManagementError::invalid("seed_binding_not_found", "seed binding was not found")
}

pub(super) fn submission_conflict() -> ManagementError {
    ManagementError::conflict(
        "seed_submission_conflict",
        "request_id is already bound to a different actor or request",
    )
}

pub(super) fn corrupt_seed_record(message: &str) -> ManagementError {
    ManagementError::store("seed_binding_corrupt", message)
}
