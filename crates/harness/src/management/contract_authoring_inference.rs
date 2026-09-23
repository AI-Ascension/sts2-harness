// SPDX-License-Identifier: MIT

//! Closed, credential-free contracts for bounded proposal-only authoring
//! inference (`sts2-harness#105`).
//!
//! The request binds an immutable base (draft revision/etag/definition digest),
//! the exact catalog revisions the proposal must be resolved from, one pinned
//! inference-profile reference, a bounded requirement and an explicit
//! generation budget. The response is a *proposal*: a candidate definition plus
//! its exact base/catalog/provenance bindings. Nothing in this contract can
//! publish a definition, start a run or reach a game instance; those remain
//! separate harness authorities the endpoint never invokes.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::contract::{ContractError, Diagnostic, validate_digest, validate_identifier};

pub const AUTHORING_INFERENCE_REQUEST_SCHEMA_VERSION: &str =
    "ascension.authoring-inference-request/v1";
pub const AUTHORING_INFERENCE_PROPOSAL_SCHEMA_VERSION: &str =
    "ascension.authoring-inference-proposal/v1";
pub const AUTHORING_INFERENCE_OPERATION_SCHEMA_VERSION: &str =
    "ascension.authoring-inference-operation/v1";

/// Maximum bytes of the caller-supplied requirement summary.
pub const MAX_AUTHORING_REQUIREMENT_BYTES: usize = 2 * 1024;
/// Maximum bytes of the candidate definition a provider may return.
pub const MAX_AUTHORING_CANDIDATE_BYTES: u64 = 256 * 1024;
/// Maximum provider calls one authoring operation may reserve.
pub const MAX_AUTHORING_PROVIDER_CALLS: u64 = 8;
/// Maximum output tokens one authoring operation may reserve.
pub const MAX_AUTHORING_OUTPUT_TOKENS: u64 = 64 * 1024;
/// Maximum stages one requirement may ask for.
pub const MAX_AUTHORING_STAGES: u64 = 32;
/// Maximum unsatisfied requirements retained on a proposal.
pub const MAX_AUTHORING_UNSATISFIED: usize = 32;
/// Maximum diagnostics retained on a proposal.
pub const MAX_AUTHORING_DIAGNOSTICS: usize = 64;

/// The immutable base a proposal is authored against.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AuthoringInferenceBase {
    pub draft_id: String,
    pub revision: u64,
    pub etag: String,
    pub definition_digest: String,
}

/// The exact catalog revisions a proposal must be resolved from.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AuthoringInferenceCatalogs {
    pub inference_catalog_digest: String,
    pub capability_manifest_digest: String,
}

/// A bounded statement of what the caller wants authored.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AuthoringInferenceRequirement {
    pub summary: String,
    pub max_stages: u64,
}

/// The explicit generation budget the operation may reserve.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AuthoringInferenceBudget {
    pub max_provider_calls: u64,
    pub max_output_tokens: u64,
    pub max_candidate_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AuthoringInferenceRequest {
    pub schema_version: String,
    pub client_mutation_id: String,
    /// `profile_id:major.minor.patch:<sha256>`; never a floating id, so the
    /// provenance binding is exact before any provider call.
    pub inference_profile_ref: String,
    pub base: AuthoringInferenceBase,
    pub catalogs: AuthoringInferenceCatalogs,
    pub requirement: AuthoringInferenceRequirement,
    pub budget: AuthoringInferenceBudget,
}

/// Honestly measured cost of one authoring operation.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AuthoringInferenceCost {
    pub provider_calls: u64,
    pub output_tokens: u64,
}

/// The exact bindings a proposal was produced under, sealed by digest.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AuthoringInferenceProvenance {
    pub operation_id: String,
    pub request_digest: String,
    pub base_revision: u64,
    pub base_etag: String,
    pub base_definition_digest: String,
    pub inference_catalog_digest: String,
    pub capability_manifest_digest: String,
    pub inference_profile_ref: String,
    pub compiler: String,
    pub requirement_digest: String,
    pub candidate_digest: String,
    pub operation_digest: String,
}

/// One proposal-only response. It carries a candidate definition and its exact
/// bindings; it is never a published definition and never a run.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AuthoringInferenceProposal {
    pub schema_version: String,
    /// Always `proposed`; a refused operation is a typed error, not a proposal.
    pub outcome: String,
    pub proposal_id: String,
    pub base: AuthoringInferenceBase,
    pub catalogs: AuthoringInferenceCatalogs,
    pub provenance: AuthoringInferenceProvenance,
    pub definition: Value,
    pub unsatisfied: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
    pub cost: AuthoringInferenceCost,
}

/// The one durable state of an authoring operation.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthoringInferenceOperationState {
    /// Reserved, result not yet known.
    Pending,
    /// A proposal was returned.
    Proposed,
    /// The candidate failed admission.
    Refused,
    /// The declared budget was exhausted.
    BudgetExhausted,
    /// The operation was cancelled before a proposal existed.
    Cancelled,
    /// The provider outcome could not be determined; never automatically repeated.
    Unknown,
}

impl AuthoringInferenceOperationState {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        !matches!(self, Self::Pending)
    }
}

/// The durable record of one authoring operation.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AuthoringInferenceOperationRecord {
    pub schema_version: String,
    pub operation_id: String,
    pub draft_id: String,
    pub client_mutation_id: String,
    pub request_digest: String,
    pub state: AuthoringInferenceOperationState,
    pub cost: AuthoringInferenceCost,
    pub proposal_id: Option<String>,
    /// The proposal body recorded for a `proposed` operation, so a retry of the
    /// same operation replays it instead of contacting the provider again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal: Option<Box<AuthoringInferenceProposal>>,
    /// Bounded, redacted operator explanation; never the provider payload.
    pub detail: String,
}

impl AuthoringInferenceRequest {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema_version != AUTHORING_INFERENCE_REQUEST_SCHEMA_VERSION {
            return Err(ContractError::new(
                "unsupported_schema",
                format!("expected schema {AUTHORING_INFERENCE_REQUEST_SCHEMA_VERSION}"),
            ));
        }
        validate_identifier("client_mutation_id", &self.client_mutation_id)?;
        validate_identifier("inference_profile_ref", &self.inference_profile_ref)?;
        validate_identifier("draft_id", &self.base.draft_id)?;
        validate_digest("etag", &self.base.etag)?;
        validate_digest("definition_digest", &self.base.definition_digest)?;
        validate_digest(
            "inference_catalog_digest",
            &self.catalogs.inference_catalog_digest,
        )?;
        validate_digest(
            "capability_manifest_digest",
            &self.catalogs.capability_manifest_digest,
        )?;
        validate_requirement(&self.requirement)?;
        validate_budget(&self.budget)
    }
}

fn validate_requirement(requirement: &AuthoringInferenceRequirement) -> Result<(), ContractError> {
    if requirement.summary.is_empty()
        || requirement.summary.len() > MAX_AUTHORING_REQUIREMENT_BYTES
        || requirement.max_stages == 0
        || requirement.max_stages > MAX_AUTHORING_STAGES
    {
        return Err(ContractError::new(
            "authoring_inference_requirement_invalid",
            "the authoring requirement is outside its bounds",
        ));
    }
    Ok(())
}

fn validate_budget(budget: &AuthoringInferenceBudget) -> Result<(), ContractError> {
    if budget.max_provider_calls == 0
        || budget.max_provider_calls > MAX_AUTHORING_PROVIDER_CALLS
        || budget.max_output_tokens == 0
        || budget.max_output_tokens > MAX_AUTHORING_OUTPUT_TOKENS
        || budget.max_candidate_bytes == 0
        || budget.max_candidate_bytes > MAX_AUTHORING_CANDIDATE_BYTES
    {
        return Err(ContractError::new(
            "authoring_inference_budget_invalid",
            "the authoring generation budget is outside its bounds",
        ));
    }
    Ok(())
}
