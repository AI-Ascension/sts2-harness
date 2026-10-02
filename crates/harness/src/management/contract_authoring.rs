// SPDX-License-Identifier: MIT

//! Additive Studio authoring contracts. These are control-plane records only;
//! compilation, publication admission and execution remain harness-owned.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::contract::ResolvedInferenceProfileRef;

pub const STUDIO_SCHEMA_VERSION: &str = "ascension.studio-authoring/v1";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StudioDefinitionRecord {
    pub schema_version: String,
    pub id: String,
    pub title: String,
    pub description: String,
    pub source: String,
    pub version: String,
    pub definition_digest: String,
    pub definition: Value,
    pub published_revision: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StudioDraftConflict {
    pub server_revision: u64,
    pub server_etag: String,
    pub server_document: Value,
    pub server_layout: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StudioDraftRecord {
    pub schema_version: String,
    pub draft_id: String,
    pub definition_id: String,
    pub revision: u64,
    pub etag: String,
    pub document: Value,
    pub layout: Value,
    pub updated_at: String,
    pub conflict: Option<StudioDraftConflict>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StudioDefinitionsResponse {
    pub schema_version: String,
    pub definitions: Vec<StudioDefinitionRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StudioCreateDraftRequest {
    pub schema_version: String,
    pub draft_id: String,
    pub definition_id: String,
    pub document: Value,
    pub layout: Value,
    pub client_mutation_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StudioSaveDraftRequest {
    pub schema_version: String,
    pub expected_revision: u64,
    pub etag: String,
    pub client_mutation_id: String,
    pub document: Value,
    pub layout: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StudioPublishDraftRequest {
    pub schema_version: String,
    pub expected_revision: u64,
    pub etag: String,
    pub client_mutation_id: String,
    pub expected_definition_digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StudioPublishResponse {
    pub schema_version: String,
    pub outcome: String,
    pub definition: Option<StudioDefinitionRecord>,
    pub draft: Option<StudioDraftRecord>,
    /// The owner's authoritative inference-profile decision for the published
    /// document, with each authored reference resolved to its exact
    /// `profile_id:version:digest` pin.
    ///
    /// Publication refuses before this is returned when a reference does not
    /// resolve. `None` means the owner serves no inference-profile catalog, so
    /// no profile authority was exercised; an empty vector means the catalog was
    /// served and the document carries no inference-profile reference.
    pub inference_profiles: Option<Vec<ResolvedInferenceProfileRef>>,
}
