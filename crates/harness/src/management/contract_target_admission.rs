// SPDX-License-Identifier: MIT

//! Closed, redacted target discovery and admission contracts.
//!
//! These values describe control-plane identity only.  They intentionally do
//! not carry executable paths, credentials, leases, or provider payloads.

use serde::{Deserialize, Serialize};

use super::json::{
    ContractError, MAX_IDENTIFIER_BYTES, digest_value, validate_digest, validate_identifier,
};

pub const TARGET_CATALOG_SCHEMA_VERSION: &str = "ascension.workflow-targets/v1";
pub const TARGET_ADMISSION_SCHEMA_VERSION: &str = "ascension.workflow-admission/v1";
pub const MAX_TARGETS: usize = 32;
pub const MAX_TARGET_PROFILES: usize = 32;
pub const MAX_TARGET_CAPABILITIES: usize = 256;
pub const MAX_TARGET_OPERATIONS: usize = 64;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    Synthetic,
    Live,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TargetAvailability {
    Available,
    Unavailable,
    Revoked,
    Expired,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TargetDescriptor {
    pub instance_id: String,
    pub execution_profiles: Vec<String>,
    pub execution_mode: ExecutionMode,
    pub compatibility_revision: String,
    pub capability_revision: String,
    pub availability: TargetAvailability,
    pub supported_operations: Vec<String>,
    pub capabilities: Vec<String>,
    pub game_profiles: Vec<String>,
    pub save_profiles: Vec<String>,
    pub inference_profiles: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TargetCatalogResponse {
    pub schema_version: String,
    pub catalog_revision: String,
    pub targets: Vec<TargetDescriptor>,
}

impl TargetDescriptor {
    pub fn validate(&self) -> Result<(), ContractError> {
        validate_identifier("instance_id", &self.instance_id)?;
        validate_unique_identifiers(
            "execution_profiles",
            &self.execution_profiles,
            MAX_TARGET_PROFILES,
        )?;
        validate_identifier("compatibility_revision", &self.compatibility_revision)?;
        validate_identifier("capability_revision", &self.capability_revision)?;
        validate_unique_identifiers(
            "supported_operations",
            &self.supported_operations,
            MAX_TARGET_OPERATIONS,
        )?;
        validate_unique_identifiers("capabilities", &self.capabilities, MAX_TARGET_CAPABILITIES)?;
        validate_unique_identifiers("game_profiles", &self.game_profiles, MAX_TARGET_PROFILES)?;
        validate_unique_identifiers("save_profiles", &self.save_profiles, MAX_TARGET_PROFILES)?;
        validate_unique_identifiers(
            "inference_profiles",
            &self.inference_profiles,
            MAX_TARGET_PROFILES,
        )?;
        if self.execution_profiles.is_empty()
            || self.game_profiles.is_empty()
            || self.capabilities.is_empty()
        {
            return Err(ContractError::new(
                "target_descriptor_incomplete",
                "target descriptor must declare profiles and capabilities",
            ));
        }
        Ok(())
    }

    pub fn digest(&self) -> Result<String, ContractError> {
        let value = serde_json::to_value(self)
            .map_err(|error| ContractError::new("target_descriptor_encode", error.to_string()))?;
        digest_value(&value)
    }
}

impl TargetCatalogResponse {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema_version != TARGET_CATALOG_SCHEMA_VERSION {
            return Err(ContractError::new(
                "target_catalog_schema",
                "target catalog schema version is unsupported",
            ));
        }
        validate_identifier("catalog_revision", &self.catalog_revision)?;
        if self.targets.len() > MAX_TARGETS {
            return Err(ContractError::new(
                "target_catalog_capacity",
                "target catalog exceeds the supported bound",
            ));
        }
        let mut instance_ids = std::collections::BTreeSet::new();
        for target in &self.targets {
            target.validate()?;
            if !instance_ids.insert(target.instance_id.as_str()) {
                return Err(ContractError::new(
                    "target_catalog_duplicate",
                    "target catalog instance IDs must be unique",
                ));
            }
        }
        Ok(())
    }
}

/// Exact target/profile information carried with a run request.
///
/// Profile namespaces remain separate so a consumer cannot accidentally treat
/// a game profile as a save or inference profile.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RunTargetConfiguration {
    pub instance_id: String,
    pub execution_profile: String,
    pub execution_mode: ExecutionMode,
    pub workflow_revision: String,
    pub compatibility_revision: String,
    pub capability_revision: String,
    pub game_profile: String,
    pub save_profile: Option<String>,
    pub inference_profile: Option<String>,
    pub context_capability: Option<String>,
    pub provider_capability: Option<String>,
}

impl RunTargetConfiguration {
    pub fn validate(&self) -> Result<(), ContractError> {
        validate_identifier("instance_id", &self.instance_id)?;
        validate_identifier("execution_profile", &self.execution_profile)?;
        validate_identifier("workflow_revision", &self.workflow_revision)?;
        validate_identifier("compatibility_revision", &self.compatibility_revision)?;
        validate_identifier("capability_revision", &self.capability_revision)?;
        validate_identifier("game_profile", &self.game_profile)?;
        for (field, value) in [
            ("save_profile", self.save_profile.as_deref()),
            ("inference_profile", self.inference_profile.as_deref()),
            ("context_capability", self.context_capability.as_deref()),
            ("provider_capability", self.provider_capability.as_deref()),
        ] {
            if let Some(value) = value {
                validate_identifier(field, value)?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TargetAdmissionRequest {
    pub schema_version: String,
    pub request_id: String,
    pub workflow_definition_digest: String,
    pub target: RunTargetConfiguration,
}

impl TargetAdmissionRequest {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema_version != TARGET_ADMISSION_SCHEMA_VERSION {
            return Err(ContractError::new(
                "target_admission_schema",
                "target admission schema version is unsupported",
            ));
        }
        validate_identifier("request_id", &self.request_id)?;
        validate_digest(
            "workflow_definition_digest",
            &self.workflow_definition_digest,
        )?;
        self.target.validate()
    }
}

/// The server's exact, revalidated admission binding.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TargetAdmissionBinding {
    pub schema_version: String,
    pub request_id: String,
    pub workflow_definition_digest: String,
    pub target: RunTargetConfiguration,
    pub descriptor_digest: String,
    pub catalog_revision: String,
}

impl TargetAdmissionBinding {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema_version != TARGET_ADMISSION_SCHEMA_VERSION {
            return Err(ContractError::new(
                "target_admission_schema",
                "target admission schema version is unsupported",
            ));
        }
        validate_identifier("request_id", &self.request_id)?;
        validate_digest(
            "workflow_definition_digest",
            &self.workflow_definition_digest,
        )?;
        self.target.validate()?;
        validate_digest("descriptor_digest", &self.descriptor_digest)?;
        validate_identifier("catalog_revision", &self.catalog_revision)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TargetPreflightResponse {
    pub schema_version: String,
    pub admission: TargetAdmissionBinding,
}

impl TargetPreflightResponse {
    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema_version != TARGET_ADMISSION_SCHEMA_VERSION {
            return Err(ContractError::new(
                "target_admission_schema",
                "target admission schema version is unsupported",
            ));
        }
        self.admission.validate()
    }
}

fn validate_unique_identifiers(
    field: &str,
    values: &[String],
    max: usize,
) -> Result<(), ContractError> {
    if values.len() > max {
        return Err(ContractError::new(
            "target_descriptor_capacity",
            format!("{field} exceeds the supported bound"),
        ));
    }
    let mut unique = std::collections::BTreeSet::new();
    for value in values {
        if value.len() > MAX_IDENTIFIER_BYTES {
            return Err(ContractError::new(
                "invalid_identifier",
                format!("{field} contains an oversized identifier"),
            ));
        }
        validate_identifier(field, value)?;
        if !unique.insert(value.as_str()) {
            return Err(ContractError::new(
                "target_descriptor_duplicate",
                format!("{field} must contain unique identifiers"),
            ));
        }
    }
    Ok(())
}
