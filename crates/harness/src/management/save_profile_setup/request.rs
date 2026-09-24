// SPDX-License-Identifier: MIT

//! The authored save-profile setup request and the deployment's grants.
//!
//! The request is closed: the schema version is pinned, unknown members are
//! refused at decode time, and every identity is a bounded portable identifier.
//! The grants are supplied by the deployment, not by the request, so an authored
//! workflow cannot widen its own permission by declaring one.

use serde::{Deserialize, Serialize};

use super::{ProfileSetupError, ProfileSetupOperation};

/// The setup-request schema version this harness implements.
pub const PROFILE_SETUP_SCHEMA_VERSION: &str = "ascension.save-profile-setup/v1";

/// Maximum bytes of a save-profile identity.
pub const MAX_PROFILE_ID_BYTES: usize = 128;

/// An admitted baseline a selection must fence against.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileBaselineFence {
    /// The profile identity the baseline was read for.
    pub profile_id: String,
    /// Lowercase SHA-256 digest of the admitted baseline.
    pub baseline_digest: String,
}

impl ProfileBaselineFence {
    /// Validates the fence's profile identity and digest shape.
    pub fn validate(&self) -> Result<(), ProfileSetupError> {
        if !is_profile_identity(&self.profile_id) {
            return Err(ProfileSetupError::InvalidRequest);
        }
        if self.baseline_digest.len() != 64
            || !self
                .baseline_digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(ProfileSetupError::InvalidRequest);
        }
        Ok(())
    }
}

/// The permissions one deployment grants an authored workflow.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileSetupGrants {
    /// Whether effect-free discovery is granted.
    pub discovery: bool,
    /// Whether selecting an existing profile is granted.
    pub selection: bool,
    /// Whether requesting a disposable profile is granted.
    pub provisioning: bool,
}

impl ProfileSetupGrants {
    /// Grants nothing; the deployment must widen this explicitly.
    #[must_use]
    pub const fn none() -> Self {
        Self {
            discovery: false,
            selection: false,
            provisioning: false,
        }
    }

    /// Whether this grant set permits `operation`.
    #[must_use]
    pub const fn permits(self, operation: ProfileSetupOperation) -> bool {
        match operation.required_grant() {
            super::ProfileGrant::Discovery => self.discovery,
            super::ProfileGrant::Selection => self.selection,
            super::ProfileGrant::Provisioning => self.provisioning,
        }
    }
}

/// One authored save-profile setup operation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileSetupRequest {
    /// The setup schema version this request was authored against.
    pub schema_version: String,
    /// The instance the operation targets.
    pub instance_id: String,
    /// The operation to map.
    pub operation: ProfileSetupOperationDocument,
    /// The profile to select, when the operation selects one.
    pub profile_id: Option<String>,
    /// The baseline fence a selection must carry.
    pub baseline: Option<ProfileBaselineFence>,
    /// The retained operation identity a receipt read names.
    pub operation_id: Option<String>,
    /// Whether the run this setup belongs to is already active.
    pub run_active: bool,
}

/// The serialized operation discriminator.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileSetupOperationDocument {
    /// List bounded save-profile summaries.
    List,
    /// Read the current save-profile summary.
    Current,
    /// Read one retained operation receipt.
    Status,
    /// Select one existing profile.
    Select,
    /// Request one isolated disposable profile.
    CreateDisposable,
}

impl ProfileSetupOperationDocument {
    /// The operation this document names.
    #[must_use]
    pub const fn operation(self) -> ProfileSetupOperation {
        match self {
            Self::List => ProfileSetupOperation::List,
            Self::Current => ProfileSetupOperation::Current,
            Self::Status => ProfileSetupOperation::Status,
            Self::Select => ProfileSetupOperation::Select,
            Self::CreateDisposable => ProfileSetupOperation::CreateDisposable,
        }
    }
}

/// Whether `value` is a bounded, portable save-profile identity.
///
/// The shape refuses a path traversal, a URL and a host path, so an authored
/// identity cannot become a filesystem or network reference.
#[must_use]
pub fn is_profile_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_PROFILE_ID_BYTES
        && !value.contains("..")
        && !value.contains("://")
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | ':' | '-')
        })
}

/// The instance identity is held to the same portable shape as a profile.
#[must_use]
pub fn is_instance_identity(value: &str) -> bool {
    is_profile_identity(value)
}

pub(crate) fn validate_request(request: &ProfileSetupRequest) -> Result<(), ProfileSetupError> {
    if request.schema_version != PROFILE_SETUP_SCHEMA_VERSION {
        return Err(ProfileSetupError::Incompatible);
    }
    if !is_instance_identity(&request.instance_id) {
        return Err(ProfileSetupError::InvalidRequest);
    }
    if let Some(profile_id) = request.profile_id.as_deref()
        && !is_profile_identity(profile_id)
    {
        return Err(ProfileSetupError::InvalidRequest);
    }
    if let Some(operation_id) = request.operation_id.as_deref()
        && !is_profile_identity(operation_id)
    {
        return Err(ProfileSetupError::InvalidRequest);
    }
    if let Some(baseline) = request.baseline.as_ref() {
        baseline.validate()?;
    }
    Ok(())
}
