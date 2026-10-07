// SPDX-License-Identifier: MIT

use super::super::auth::AuthContext;
use super::super::contract::{DiagnosticSeverity, RunSnapshot, TargetAvailability, digest_value};
use super::super::contract_seed_v2::SeedModeV2;
use super::super::contract_seed_v2::{
    SeedBindingReadbackV2, SeedBindingStateV2, SeededRunSubmissionResponseV2,
    StoredSeedOperationV2, WORKFLOW_RUN_SUBMISSION_V2_SCHEMA, WorkflowRunRequestV2,
};
use super::super::seed_key::SeedDerivationKeyReadiness;
use super::super::seed_key::SeedKeyError;
use super::super::seed_v2_crypto::{
    derive_candidate_from_operation, load_pinned_operation_key, verify_candidate,
};
use super::super::store::SeedBindingRecord;
use super::support::authorize;
use super::support::{run_submission_response, verify_digest};
use super::{ManagementError, ManagementService};

pub(super) const TARGET_SEED_SUPPORT_REVISION: &str = "seed-support.v1";

pub(super) fn supports_durable_candidate_mode(
    service: &ManagementService,
    mode: SeedModeV2,
) -> bool {
    if !service.store.supports_durable_seed_bindings() {
        return false;
    }
    match mode {
        SeedModeV2::Explicit => true,
        SeedModeV2::DeriveOnce => {
            service.store.supports_seed_operation_reservations()
                && service.seed_derivation_keys().is_some()
        }
    }
}

pub(super) fn durable_candidate_modes(service: &ManagementService) -> Vec<SeedModeV2> {
    [SeedModeV2::Explicit, SeedModeV2::DeriveOnce]
        .into_iter()
        .filter(|mode| supports_durable_candidate_mode(service, *mode))
        .collect()
}

pub(super) fn ready_candidate_modes(
    service: &ManagementService,
    availability: &TargetAvailability,
    durable_modes: &[SeedModeV2],
) -> Vec<SeedModeV2> {
    if availability != &TargetAvailability::Available {
        return Vec::new();
    }
    durable_modes
        .iter()
        .copied()
        .filter(|mode| match mode {
            SeedModeV2::Explicit => true,
            SeedModeV2::DeriveOnce => service.seed_derivation_keys().is_some_and(|authority| {
                authority.readiness() == SeedDerivationKeyReadiness::Ready
            }),
        })
        .collect()
}

pub(super) fn require_durable_candidate_mode(
    service: &ManagementService,
    mode: SeedModeV2,
) -> Result<(), ManagementError> {
    if supports_durable_candidate_mode(service, mode) {
        return Ok(());
    }
    Err(ManagementError::unavailable(
        "seed_candidate_support_unavailable",
        "configured storage and key authority cannot durably support this seed mode",
    ))
}

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
        Ok(digest_value(
            &serde_json::json!({ "artifact_id": artifact_id }),
        )?)
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

pub(super) fn existing_submission(
    service: &ManagementService,
    actor: &AuthContext,
    request: &WorkflowRunRequestV2,
    request_digest: &str,
    record: SeedBindingRecord,
) -> Result<SeededRunSubmissionResponseV2, ManagementError> {
    let record = record.record();
    let workflow_run_id = &record.workflow_run_id;
    authorize(actor, "workflow:control", Some(workflow_run_id))?;
    let snapshot = service
        .store
        .get_run(workflow_run_id)?
        .ok_or_else(|| corrupt_seed_record("seed binding has no workflow snapshot"))?;
    if snapshot.workflow_run_id != *workflow_run_id {
        return Err(corrupt_seed_record(
            "workflow snapshot identity does not match its seed binding",
        ));
    }
    if !verify_candidate(
        record,
        request,
        actor,
        request_digest,
        workflow_run_id,
        service.seed_derivation_keys(),
    )
    .map_err(seed_key_error)?
    {
        return Err(corrupt_seed_record(
            "stored seed binding failed its owner and key verification",
        ));
    }
    let stored = if record.state == SeedBindingStateV2::CandidatePersisted {
        service.store.mark_seed_binding_awaiting_host_context(
            workflow_run_id,
            &record.actor_digest,
            request_digest,
        )?
    } else {
        SeedBindingRecord::new(record.clone())
    };
    let readback = service
        .store
        .read_seed_binding(workflow_run_id)?
        .ok_or_else(|| corrupt_seed_record("seed binding disappeared during replay"))?;
    if stored.record() != readback.record() {
        return Err(corrupt_seed_record(
            "seed binding changed during replay readback",
        ));
    }
    let readback = readback.record();
    if !verify_candidate(
        readback,
        request,
        actor,
        request_digest,
        workflow_run_id,
        service.seed_derivation_keys(),
    )
    .map_err(seed_key_error)?
    {
        return Err(corrupt_seed_record(
            "replayed seed candidate failed pinned-key verification",
        ));
    }
    Ok(response(&snapshot, readback.readback()))
}

pub(super) fn derive_from_pinned_operation(
    service: &ManagementService,
    operation: &StoredSeedOperationV2,
    actor: &AuthContext,
    request: &WorkflowRunRequestV2,
    request_digest: &str,
) -> Result<super::super::contract_seed_v2::StoredSeedBindingV2, ManagementError> {
    let key = load_pinned_operation_key(operation, service.seed_derivation_keys())
        .map_err(seed_key_error)?;
    derive_candidate_from_operation(request, actor, request_digest, operation, &key)
        .map_err(seed_key_error)
}

/// A contender's provisional current key may differ from the immutable winner.
/// Compare operation identity and admitted configuration here; the winner's
/// key is independently reloaded and checked before derivation.
pub(super) fn same_operation_request(
    proposed: &StoredSeedOperationV2,
    stored: &StoredSeedOperationV2,
) -> bool {
    proposed.schema_version == stored.schema_version
        && proposed.request_id == stored.request_id
        && proposed.actor_digest == stored.actor_digest
        && proposed.request_digest == stored.request_digest
        && proposed.workflow_run_id == stored.workflow_run_id
        && proposed.operation_id == stored.operation_id
        && proposed.mode == stored.mode
        && proposed.admitted_configuration == stored.admitted_configuration
        && proposed.configuration_digest == stored.configuration_digest
        && proposed.derivation.algorithm_id == stored.derivation.algorithm_id
}
