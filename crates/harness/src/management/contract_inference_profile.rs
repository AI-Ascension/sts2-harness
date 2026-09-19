// SPDX-License-Identifier: MIT

//! Closed, credential-free inference-profile catalog contracts.
//!
//! A descriptor names one owner-resolved decision or planner configuration by
//! exact identity (`profile_id`, `version`, `digest`) and publishes only bounded
//! metadata: adapter, declared and observed model identity, prompt and settings
//! revisions, supported settings and operations, context compatibility,
//! continuity, effective budgets, grants and availability.  Credentials,
//! executables, URLs, prompt bytes and tool authority never enter this
//! contract; they stay behind the owner that serves the provider.

use serde::{Deserialize, Serialize};

use super::json::{ContractError, validate_digest, validate_identifier};
use crate::sha256_hex;
use crate::workflow::{RegistryId, SemanticVersion};

pub const INFERENCE_PROFILE_CATALOG_SCHEMA_VERSION: &str = "ascension.inference-profiles/v1";
pub const INFERENCE_PROFILE_SCHEMA_VERSION: &str = "ascension.inference-profile/v1";
pub const MAX_INFERENCE_PROFILES: usize = 64;
pub const MAX_INFERENCE_PROFILE_NODE_KINDS: usize = 8;
pub const MAX_INFERENCE_PROFILE_OPERATIONS: usize = 32;
pub const MAX_INFERENCE_PROFILE_SETTINGS: usize = 32;
pub const MAX_INFERENCE_PROFILE_CONTEXTS: usize = 32;
/// Matches the provider registry's input ceiling so both exact gates agree.
pub const MAX_INFERENCE_INPUT_BYTES: u64 = 128 * 1024;
/// Matches the provider registry's output ceiling.
pub const MAX_INFERENCE_OUTPUT_TOKENS: u64 = 2_000_000;
pub const MAX_INFERENCE_PROVIDER_CALLS: u64 = 10_000;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InferenceProfileState {
    Available,
    Disabled,
    Revoked,
    Stale,
    Unsupported,
}

/// Selection and edit are separate permissions. Edit is published so a consumer
/// can render a capability-driven unavailable state; the revision-edit route
/// itself is a later delivery.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InferenceProfileGrants {
    pub select: bool,
    pub edit: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InferenceProfileContinuity {
    pub provider_session_continuity: bool,
    pub survives_controller_restart: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InferenceProfileBudgets {
    pub max_input_bytes: u64,
    pub max_output_tokens: u64,
    pub max_provider_calls: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InferenceProfileDescriptor {
    pub schema_version: String,
    pub profile_id: String,
    /// `major.minor.patch`; together with `profile_id` this is the registry key.
    pub version: String,
    pub digest: String,
    pub adapter: String,
    pub requested_model: String,
    /// Observed effective model identity. `None` is the honest value while the
    /// owner has not observed one; it is never inferred from `requested_model`.
    pub resolved_model: Option<String>,
    pub prompt_revision: String,
    pub settings_revision: String,
    pub supported_settings: Vec<String>,
    pub operations: Vec<String>,
    pub node_kinds: Vec<String>,
    /// Context references this profile accepts. Empty means unconstrained.
    pub context_compatibility: Vec<String>,
    pub continuity: InferenceProfileContinuity,
    pub effective_budgets: InferenceProfileBudgets,
    pub grants: InferenceProfileGrants,
    pub state: InferenceProfileState,
}

impl InferenceProfileDescriptor {
    /// Seals the immutable fields under a fresh digest.
    pub fn seal(mut self) -> Result<Self, ContractError> {
        self.digest.clear();
        let bytes = serde_json::to_vec(&self)
            .map_err(|error| ContractError::new("inference_profile_encode", error.to_string()))?;
        self.digest = sha256_hex(bytes);
        Ok(self)
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema_version != INFERENCE_PROFILE_SCHEMA_VERSION
            || self.node_kinds.is_empty()
            || self.node_kinds.len() > MAX_INFERENCE_PROFILE_NODE_KINDS
            || self.operations.len() > MAX_INFERENCE_PROFILE_OPERATIONS
            || self.supported_settings.len() > MAX_INFERENCE_PROFILE_SETTINGS
            || self.context_compatibility.len() > MAX_INFERENCE_PROFILE_CONTEXTS
        {
            return Err(ContractError::new(
                "inference_profile_descriptor_invalid",
                "inference profile descriptor is outside its bounds",
            ));
        }
        for (field, value) in [
            ("profile_id", self.profile_id.as_str()),
            ("adapter", self.adapter.as_str()),
            ("requested_model", self.requested_model.as_str()),
            ("prompt_revision", self.prompt_revision.as_str()),
            ("settings_revision", self.settings_revision.as_str()),
        ] {
            validate_identifier(field, value)?;
        }
        if let Some(model) = self.resolved_model.as_deref() {
            validate_identifier("resolved_model", model)?;
        }
        RegistryId::new(self.profile_id.as_str())
            .map_err(|error| ContractError::new("inference_profile_id", error.to_string()))?;
        SemanticVersion::new(self.version.as_str())
            .map_err(|error| ContractError::new("inference_profile_version", error.to_string()))?;
        validate_digest("inference_profile_digest", &self.digest)?;
        validate_unique_identifiers("node_kinds", &self.node_kinds)?;
        validate_unique_identifiers("operations", &self.operations)?;
        validate_unique_identifiers("supported_settings", &self.supported_settings)?;
        validate_unique_identifiers("context_compatibility", &self.context_compatibility)?;
        validate_budgets(&self.effective_budgets)?;
        // Only hash after every nested field has been validated and bounded.
        let expected = self.clone().seal()?.digest;
        if expected != self.digest {
            return Err(ContractError::new(
                "inference_profile_digest_mismatch",
                "inference profile digest does not match its immutable fields",
            ));
        }
        Ok(())
    }

    /// Whether this descriptor can serve `node_kind` for `profile_id` right
    /// now. Anything but `Available` is discoverable metadata, never a binding.
    pub fn supports(&self, profile_id: &str, node_kind: &str) -> bool {
        self.profile_id == profile_id
            && self.node_kinds.iter().any(|kind| kind == node_kind)
            && self.state == InferenceProfileState::Available
    }
}

fn validate_unique_identifiers(field: &str, values: &[String]) -> Result<(), ContractError> {
    let mut seen = std::collections::BTreeSet::new();
    for value in values {
        validate_identifier(field, value)?;
        if !seen.insert(value) {
            return Err(ContractError::new(
                "inference_profile_duplicate_entry",
                format!("{field} entries must be unique"),
            ));
        }
    }
    Ok(())
}

fn validate_budgets(budgets: &InferenceProfileBudgets) -> Result<(), ContractError> {
    if budgets.max_input_bytes == 0
        || budgets.max_input_bytes > MAX_INFERENCE_INPUT_BYTES
        || budgets.max_output_tokens == 0
        || budgets.max_output_tokens > MAX_INFERENCE_OUTPUT_TOKENS
        || budgets.max_provider_calls == 0
        || budgets.max_provider_calls > MAX_INFERENCE_PROVIDER_CALLS
    {
        return Err(ContractError::new(
            "inference_profile_budget_invalid",
            "inference profile effective budgets are outside their bounds",
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InferenceProfileCatalog {
    pub schema_version: String,
    pub owner_id: String,
    pub owner_version: String,
    pub catalog_digest: String,
    pub descriptors: Vec<InferenceProfileDescriptor>,
}

impl InferenceProfileCatalog {
    /// Seals the catalog encoding without changing its descriptors. Call
    /// `validate` separately; a self-consistent digest is not owner authority.
    pub fn seal(mut self) -> Result<Self, ContractError> {
        self.catalog_digest =
            inference_catalog_digest(&self.owner_id, &self.owner_version, &self.descriptors)?;
        Ok(self)
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema_version != INFERENCE_PROFILE_CATALOG_SCHEMA_VERSION
            || self.descriptors.len() > MAX_INFERENCE_PROFILES
        {
            return Err(ContractError::new(
                "inference_profile_catalog_invalid",
                "inference profile catalog is outside its bounds",
            ));
        }
        validate_identifier("inference_owner_id", &self.owner_id)?;
        validate_identifier("inference_owner_version", &self.owner_version)?;
        validate_digest("inference_catalog_digest", &self.catalog_digest)?;
        let mut identities = std::collections::BTreeSet::new();
        for descriptor in &self.descriptors {
            descriptor.validate()?;
            if !identities.insert((&descriptor.profile_id, &descriptor.version)) {
                return Err(ContractError::new(
                    "inference_profile_duplicate",
                    "inference profile IDs and versions must be unique",
                ));
            }
        }
        let expected =
            inference_catalog_digest(&self.owner_id, &self.owner_version, &self.descriptors)?;
        if expected != self.catalog_digest {
            return Err(ContractError::new(
                "inference_catalog_digest_mismatch",
                "inference profile catalog digest does not match its descriptors",
            ));
        }
        Ok(())
    }
}

pub fn inference_catalog_digest(
    owner_id: &str,
    owner_version: &str,
    descriptors: &[InferenceProfileDescriptor],
) -> Result<String, ContractError> {
    let bytes = serde_json::to_vec(&(owner_id, owner_version, descriptors))
        .map_err(|error| ContractError::new("inference_catalog_encode", error.to_string()))?;
    Ok(sha256_hex(bytes))
}
