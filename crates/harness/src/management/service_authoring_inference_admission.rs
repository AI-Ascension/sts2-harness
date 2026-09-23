// SPDX-License-Identifier: MIT

//! Admission and sealing helpers for the proposal-only authoring-inference
//! endpoint (`sts2-harness#105`).
//!
//! Every helper here is a refusal fence, not an authority: a base, catalog or
//! candidate that does not exactly match the served revision, the owner
//! compiler and the resolved inference profiles is refused before a proposal
//! exists. Nothing here publishes, runs or reaches a game instance.

use serde_json::Value;

use crate::management::DefinitionPort;
use crate::management::authoring_inference::guard_candidate;
use crate::management::contract::{
    Diagnostic, DiagnosticSeverity, ErrorClass, InferenceProfileCatalog, digest_value,
};
use crate::management::contract_authoring_inference::{
    AUTHORING_INFERENCE_PROPOSAL_SCHEMA_VERSION, AuthoringInferenceBase, AuthoringInferenceCost,
    AuthoringInferenceOperationRecord, AuthoringInferenceOperationState,
    AuthoringInferenceProposal, AuthoringInferenceProvenance, AuthoringInferenceRequest,
};
use crate::management::inference_profile_binding::resolve_definition;
use crate::management::inference_profile_catalog::InferenceProfileRef;
use crate::management::service::ManagementError;
use crate::management::workflow_ports::parse_definition;
use crate::sha256_hex;
use crate::workflow::WORKFLOW_COMPILER_ID;

/// The exact identites a proposal is sealed against.
pub(super) struct ProposalContext<'a> {
    pub(super) request: &'a AuthoringInferenceRequest,
    pub(super) base: &'a AuthoringInferenceBase,
    pub(super) request_digest: &'a str,
    pub(super) operation_id: &'a str,
}

pub(super) fn admit_manifest(
    manifest_digest: &str,
    request: &AuthoringInferenceRequest,
) -> Result<(), ManagementError> {
    if manifest_digest != request.catalogs.capability_manifest_digest {
        return Err(ManagementError::conflict(
            "authoring_inference_manifest_stale",
            "the authored capability-manifest digest is not the served revision",
        ));
    }
    Ok(())
}

pub(super) fn admit_pinned_profile(
    catalog: &InferenceProfileCatalog,
    reference: &str,
) -> Result<(), ManagementError> {
    let parsed = InferenceProfileRef::parse(reference)?;
    let Some(pin) = parsed.pin else {
        return Err(ManagementError::invalid(
            "authoring_inference_profile_unpinned",
            "authoring inference requires an exact profile_id:version:digest reference",
        ));
    };
    let descriptor = catalog
        .descriptors
        .iter()
        .find(|descriptor| {
            descriptor.profile_id == parsed.profile_id && descriptor.version == pin.version
        })
        .ok_or_else(|| {
            ManagementError::unavailable(
                "authoring_inference_profile_unknown",
                "the served catalog does not advertise this inference profile revision",
            )
        })?;
    if descriptor.digest != pin.digest {
        return Err(ManagementError::conflict(
            "authoring_inference_profile_stale",
            "the pinned inference profile digest is not the served revision",
        ));
    }
    Ok(())
}

pub(super) fn admit_base(
    draft: &crate::management::StudioDraftRecord,
    request: &AuthoringInferenceRequest,
) -> Result<AuthoringInferenceBase, ManagementError> {
    let digest = draft_digest(&draft.document)?;
    if draft.revision != request.base.revision
        || draft.etag != request.base.etag
        || digest != request.base.definition_digest
    {
        return Err(ManagementError::conflict(
            "authoring_inference_base_conflict",
            format!(
                "draft {} is at revision {} (etag {}); re-author against the served revision",
                draft.draft_id, draft.revision, draft.etag
            ),
        ));
    }
    Ok(AuthoringInferenceBase {
        draft_id: draft.draft_id.clone(),
        revision: draft.revision,
        etag: draft.etag.clone(),
        definition_digest: digest,
    })
}

pub(super) fn admit_candidate(
    port: &dyn DefinitionPort,
    request: &AuthoringInferenceRequest,
    capabilities: &Value,
    catalog: &InferenceProfileCatalog,
    definition: &Value,
) -> Result<(), ManagementError> {
    let bytes = serde_json::to_vec(definition).map_err(|error| {
        ManagementError::invalid("authoring_inference_encode", error.to_string())
    })?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > request.budget.max_candidate_bytes {
        return Err(ManagementError::invalid(
            "authoring_inference_candidate_oversized",
            "the candidate definition exceeds the declared byte budget",
        ));
    }
    guard_candidate(definition)?;
    // The owner compiler decodes and validates the candidate exactly as it
    // would a live submission, so a forbidden node or an unavailable capability
    // is refused before any binding is sealed.
    let parsed = parse_definition(definition)?;
    let validation = port.validate(definition, capabilities)?;
    if validation
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
    {
        return Err(ManagementError::forbidden(
            "authoring_inference_candidate_refused",
            "the owner refused a forbidden node or unavailable capability in the candidate",
        ));
    }
    // Authority expansion fence: every declared decide/planner reference must
    // resolve to an exact served revision, so a proposal cannot invent or
    // escalate an inference profile.
    let _ = resolve_definition(catalog, &parsed, None)?;
    Ok(())
}

pub(super) fn seal_proposal(
    context: &ProposalContext<'_>,
    definition: Value,
    unsatisfied: Vec<String>,
    diagnostics: Vec<Diagnostic>,
    cost: AuthoringInferenceCost,
) -> Result<AuthoringInferenceProposal, ManagementError> {
    let request = context.request;
    let candidate_digest = digest_value(&definition)?;
    let requirement_digest =
        digest_value(&serde_json::to_value(&request.requirement).map_err(encode_error)?)?;
    let operation_digest = sha256_hex(
        serde_json::to_vec(&(
            context.operation_id,
            context.request_digest,
            candidate_digest.as_str(),
            context.base.definition_digest.as_str(),
            request.catalogs.inference_catalog_digest.as_str(),
            request.catalogs.capability_manifest_digest.as_str(),
            request.inference_profile_ref.as_str(),
            WORKFLOW_COMPILER_ID,
        ))
        .map_err(encode_error)?,
    );
    let provenance = AuthoringInferenceProvenance {
        operation_id: context.operation_id.to_owned(),
        request_digest: context.request_digest.to_owned(),
        base_revision: context.base.revision,
        base_etag: context.base.etag.clone(),
        base_definition_digest: context.base.definition_digest.clone(),
        inference_catalog_digest: request.catalogs.inference_catalog_digest.clone(),
        capability_manifest_digest: request.catalogs.capability_manifest_digest.clone(),
        inference_profile_ref: request.inference_profile_ref.clone(),
        compiler: WORKFLOW_COMPILER_ID.to_owned(),
        requirement_digest,
        candidate_digest,
        operation_digest: operation_digest.clone(),
    };
    Ok(AuthoringInferenceProposal {
        schema_version: AUTHORING_INFERENCE_PROPOSAL_SCHEMA_VERSION.to_owned(),
        outcome: "proposed".to_owned(),
        proposal_id: format!("authoring-proposal:{operation_digest}"),
        base: context.base.clone(),
        catalogs: request.catalogs.clone(),
        provenance,
        definition,
        unsatisfied,
        diagnostics,
        cost,
    })
}

pub(super) fn replay(
    record: &AuthoringInferenceOperationRecord,
) -> Result<AuthoringInferenceProposal, ManagementError> {
    if record.state == AuthoringInferenceOperationState::Proposed {
        if let Some(proposal) = &record.proposal {
            return Ok((**proposal).clone());
        }
        return Err(ManagementError::conflict(
            "authoring_inference_replay_unavailable",
            "the recorded proposal body is unavailable; retry with a new mutation identity",
        ));
    }
    Err(ManagementError::new(
        replay_class(record.state),
        format!(
            "authoring_inference_replayed_{}",
            replay_label(record.state)
        ),
        format!(
            "operation {} already completed with state {}: {}",
            record.operation_id,
            replay_label(record.state),
            record.detail
        ),
    ))
}

pub(super) fn draft_digest(document: &Value) -> Result<String, ManagementError> {
    digest_value(document).map_err(ManagementError::from)
}

pub(super) fn encode_error(error: serde_json::Error) -> ManagementError {
    ManagementError::invalid("authoring_inference_encode", error.to_string())
}

fn replay_label(state: AuthoringInferenceOperationState) -> &'static str {
    match state {
        AuthoringInferenceOperationState::Pending => "pending",
        AuthoringInferenceOperationState::Proposed => "proposed",
        AuthoringInferenceOperationState::Refused => "refused",
        AuthoringInferenceOperationState::BudgetExhausted => "budget_exhausted",
        AuthoringInferenceOperationState::Cancelled => "cancelled",
        AuthoringInferenceOperationState::Unknown => "unknown",
    }
}

fn replay_class(state: AuthoringInferenceOperationState) -> ErrorClass {
    match state {
        AuthoringInferenceOperationState::BudgetExhausted => ErrorClass::Budget,
        AuthoringInferenceOperationState::Cancelled => ErrorClass::Conflict,
        AuthoringInferenceOperationState::Unknown => ErrorClass::Unresolved,
        AuthoringInferenceOperationState::Refused => ErrorClass::Forbidden,
        AuthoringInferenceOperationState::Pending | AuthoringInferenceOperationState::Proposed => {
            ErrorClass::Conflict
        }
    }
}
