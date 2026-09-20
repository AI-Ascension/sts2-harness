// SPDX-License-Identifier: MIT

//! Compare-and-swap journal for accepted inference-profile revisions.
//!
//! The journal is the server-owned record of which profile revisions an owner
//! has accepted. It is append-only: an accepted edit adds a *new* revision and
//! never rewrites the revision it replaced. Adoption is therefore a
//! compare-and-swap against the exact expected revision digest, so a concurrent
//! edit loses the swap instead of silently overwriting the winner.
//!
//! An edit may change only the fields the owner published as editable:
//! `version`, `prompt_revision`, `settings_revision`, `supported_settings` and
//! `effective_budgets`. Every identity and authority field — adapter, requested
//! and resolved model, node kinds, operations, context compatibility,
//! continuity, state and grants — is inherited from the revision that was
//! edited, so a consumer edit can never widen a profile's authority. The request
//! has no field that could carry a credential, endpoint, executable or tool
//! authority, and its closed shape rejects any unadvertised field.
//!
//! The journal keeps every accepted revision, not only the newest, so the
//! immutability claim is checkable and a definition that pinned an older
//! revision still names a revision the owner has accepted.
//!
//! # What this does not do
//!
//! An accepted revision is recorded here; it is **not** spliced into the
//! owner-served catalog. The owner remains the only authority that publishes
//! what it serves, exactly as ADR 0054 requires, so an adoption cannot silently
//! change what a later admission resolves and cannot retarget a run. The
//! response names the adopted revision so the caller can author a *new*
//! definition that pins it.

use serde::{Deserialize, Serialize};

use super::contract::{
    ContractError, InferenceProfileBudgets, InferenceProfileDescriptor, InferenceProfileState,
    schema_is, validate_digest, validate_identifier,
};
use super::service::ManagementError;
use super::store::StoreError;

pub const INFERENCE_PROFILE_REVISION_SCHEMA_VERSION: &str =
    "ascension.inference-profile-revision/v1";
pub const INFERENCE_PROFILE_REVISION_RESPONSE_SCHEMA_VERSION: &str =
    "ascension.inference-profile-revision-response/v1";

/// The only fields of a served revision an admitted edit may restate.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InferenceProfileRevisionRequest {
    pub schema_version: String,
    /// The exact digest of the served revision this edit was authored against.
    pub expected_revision_digest: String,
    /// Stable caller identity, so a retried edit is replayed rather than
    /// applied twice.
    pub client_mutation_id: String,
    pub version: String,
    pub prompt_revision: String,
    pub settings_revision: String,
    pub supported_settings: Vec<String>,
    pub effective_budgets: InferenceProfileBudgets,
}

/// The admitted outcome of one edit request.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InferenceProfileRevisionResponse {
    pub schema_version: String,
    /// `adopted`, `replayed` or `conflict`.
    pub outcome: String,
    pub profile_id: String,
    /// `profile_id:version:digest` of `revision`, the exact reference a new
    /// definition pins. On a conflict this names the revision that won, so a
    /// caller can retry against it.
    pub reference: String,
    /// The adopted revision, or the currently accepted one when the swap was
    /// lost.
    pub revision: InferenceProfileDescriptor,
}

impl InferenceProfileRevisionResponse {
    pub(crate) fn new(
        outcome: &str,
        profile_id: &str,
        revision: InferenceProfileDescriptor,
    ) -> Self {
        let reference = format!("{profile_id}:{}:{}", revision.version, revision.digest);
        Self {
            schema_version: INFERENCE_PROFILE_REVISION_RESPONSE_SCHEMA_VERSION.to_owned(),
            outcome: outcome.to_owned(),
            profile_id: profile_id.to_owned(),
            reference,
            revision,
        }
    }
}

/// The result of one atomic append.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RevisionAppendOutcome {
    /// The compare-and-swap was won; `candidate` is now the accepted head.
    Adopted(Box<InferenceProfileDescriptor>),
    /// The same `client_mutation_id` and candidate were already applied.
    Replayed(Box<InferenceProfileDescriptor>),
    /// The swap was lost; the payload is the revision that won.
    Conflict(Box<InferenceProfileDescriptor>),
}

/// Server-owned append-only revision journal.
///
/// `append` must compare `expected_revision_digest` against the accepted head
/// and commit the candidate, the head pointer and the mutation record in one
/// transaction, so two concurrent edits cannot both win.
pub trait InferenceProfileRevisionJournal: Send + Sync {
    /// The currently accepted revision, or `None` before any edit was accepted.
    fn head(&self, profile_id: &str) -> Result<Option<InferenceProfileDescriptor>, StoreError>;

    /// Every accepted revision of `profile_id`, oldest first. Append-only: an
    /// entry is never removed or rewritten, so a pin to an older revision keeps
    /// naming a revision this owner accepted.
    fn history(&self, profile_id: &str) -> Result<Vec<InferenceProfileDescriptor>, StoreError>;

    /// Applies one edit as a compare-and-swap.
    ///
    /// `baseline` is the revision the owner currently serves. When the journal
    /// holds no revision for `profile_id` yet, `baseline` is recorded as the
    /// first accepted revision before the swap is evaluated, so the first edit
    /// of a freshly opened journal compares against what the owner serves
    /// rather than against nothing.
    fn append(
        &self,
        profile_id: &str,
        baseline: &InferenceProfileDescriptor,
        expected_revision_digest: &str,
        client_mutation_id: &str,
        candidate: &InferenceProfileDescriptor,
    ) -> Result<RevisionAppendOutcome, StoreError>;
}

/// Derives the new revision an admitted edit asks for.
///
/// The compare-and-swap itself is deliberately *not* evaluated here: it belongs
/// to the atomic store append, and re-reading the head in the service would
/// reintroduce the race the journal exists to close.
pub fn derive_inference_profile_revision(
    head: &InferenceProfileDescriptor,
    request: &InferenceProfileRevisionRequest,
) -> Result<InferenceProfileDescriptor, ManagementError> {
    schema_is(
        &request.schema_version,
        INFERENCE_PROFILE_REVISION_SCHEMA_VERSION,
    )?;
    validate_digest(
        "expected_revision_digest",
        &request.expected_revision_digest,
    )?;
    validate_identifier("client_mutation_id", &request.client_mutation_id)?;
    validate_identifier("version", &request.version)?;
    validate_identifier("prompt_revision", &request.prompt_revision)?;
    validate_identifier("settings_revision", &request.settings_revision)?;
    if !head.grants.edit {
        return Err(ManagementError::forbidden(
            "inference_profile_edit_denied",
            "the owner does not publish this inference profile as editable",
        ));
    }
    if head.state != InferenceProfileState::Available {
        return Err(ManagementError::conflict(
            "inference_profile_revision_state",
            "only an available inference profile revision can be edited",
        ));
    }
    // A revision is keyed by `(profile_id, version)`, so two accepted revisions
    // of one profile must carry distinct versions. An accepted edit is
    // therefore always a genuinely new identity, never a re-seal of the
    // revision it replaced.
    if request.version == head.version {
        return Err(ManagementError::invalid(
            "inference_profile_revision_version_unchanged",
            "an accepted edit must publish a new profile version",
        ));
    }
    let candidate = InferenceProfileDescriptor {
        version: request.version.clone(),
        prompt_revision: request.prompt_revision.clone(),
        settings_revision: request.settings_revision.clone(),
        supported_settings: request.supported_settings.clone(),
        effective_budgets: request.effective_budgets.clone(),
        digest: String::new(),
        ..head.clone()
    }
    .seal()?;
    candidate.validate()?;
    Ok(candidate)
}

#[path = "inference_profile_revision_memory.rs"]
mod memory;
#[path = "inference_profile_revision_sqlite.rs"]
mod sqlite;
pub use memory::{
    MemoryInferenceProfileRevisionJournal, UnavailableInferenceProfileRevisionJournal,
};
pub use sqlite::SqliteInferenceProfileRevisionJournal;

/// Rejects an append whose baseline or candidate does not belong to the profile
/// the caller named, so a journal cannot be talked into crossing profiles.
pub(crate) fn validate_baseline(
    profile_id: &str,
    baseline: &InferenceProfileDescriptor,
    candidate: &InferenceProfileDescriptor,
) -> Result<(), StoreError> {
    if baseline.profile_id != profile_id || candidate.profile_id != profile_id {
        return Err(StoreError::new(
            "inference_profile_revision_profile_mismatch",
            "the revision does not belong to the profile named by the request",
        ));
    }
    baseline.validate().map_err(request_error)?;
    candidate.validate().map_err(request_error)?;
    Ok(())
}

fn unknown() -> StoreError {
    StoreError::new(
        "inference_profile_unknown",
        "the revision journal holds no accepted revision for this inference profile",
    )
}

fn unavailable_error() -> StoreError {
    StoreError::new(
        "inference_profile_revision_journal_unavailable",
        "the inference-profile revision journal is not injected",
    )
}

fn request_error(error: ContractError) -> StoreError {
    StoreError::new("inference_profile_revision_request", error.message)
}
