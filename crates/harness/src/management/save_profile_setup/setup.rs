// SPDX-License-Identifier: MIT

//! Effect-free admission of one save-profile setup operation.
//!
//! Admission decides before any call: whether the deployment grants the
//! permission, whether discovery stayed effect-free, whether a selection is
//! properly fenced and whether a mutation conflicts with an active run. A
//! mutation that is admitted is not thereby applied — it must still be
//! reconciled against an authoritative readback, which is the only way a profile
//! becomes usable downstream.

use super::request::validate_request;
use super::{ProfileSetupError, ProfileSetupGrants, ProfileSetupOperation, ProfileSetupRequest};

/// An admitted, effect-free mapping of one setup operation.
///
/// Carries the exact tool and route the operation maps to, so the caller does
/// not re-derive either from supplied text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdmittedProfileSetup {
    operation: ProfileSetupOperation,
    instance_id: String,
    profile_id: Option<String>,
    operation_id: Option<String>,
}

impl AdmittedProfileSetup {
    /// The admitted operation.
    #[must_use]
    pub const fn operation(&self) -> ProfileSetupOperation {
        self.operation
    }

    /// The accepted MCP tool name.
    #[must_use]
    pub const fn tool(&self) -> &'static str {
        self.operation.tool()
    }

    /// Whether the admitted operation changes owner state.
    #[must_use]
    pub const fn is_mutation(&self) -> bool {
        self.operation.is_mutation()
    }

    /// The admitted instance identity.
    #[must_use]
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    /// The admitted profile identity, when the operation names one.
    #[must_use]
    pub fn profile_id(&self) -> Option<&str> {
        self.profile_id.as_deref()
    }

    /// The admitted retained operation identity, when the operation reads one.
    #[must_use]
    pub fn operation_id(&self) -> Option<&str> {
        self.operation_id.as_deref()
    }

    /// The gateway route for this admitted mapping.
    #[must_use]
    pub fn route_path(&self) -> String {
        self.operation
            .route_path(&self.instance_id, self.operation_id.as_deref())
    }
}

/// Admits one authored setup operation, refusing the first violated property.
///
/// The check order is fixed: schema and identity shape, then permission, then
/// the effect-free rule for discovery, then fence presence, then the active-run
/// rule for a mutation. The returned value is a mapping only; nothing is called.
///
/// An admitted mutation must still be reconciled: see
/// [`VerifiedProfileReadback::verify`].
pub fn admit_profile_setup(
    request: &ProfileSetupRequest,
    grants: ProfileSetupGrants,
) -> Result<AdmittedProfileSetup, ProfileSetupError> {
    validate_request(request)?;
    let operation = request.operation.operation();
    if !grants.permits(operation) {
        return Err(ProfileSetupError::PermissionDenied);
    }
    if !operation.is_mutation() && (request.profile_id.is_some() || request.baseline.is_some()) {
        // Discovery must be effect-free: a read that names a profile or a
        // baseline fence is a selection in disguise.
        return Err(ProfileSetupError::DiscoveryMustBeEffectFree);
    }
    if operation.requires_profile_id() && request.profile_id.is_none() {
        return Err(ProfileSetupError::ProfileRequired);
    }
    if operation.requires_baseline_fence() {
        let baseline = request
            .baseline
            .as_ref()
            .ok_or(ProfileSetupError::BaselineFenceMismatch)?;
        if Some(baseline.profile_id.as_str()) != request.profile_id.as_deref() {
            return Err(ProfileSetupError::BaselineFenceMismatch);
        }
    } else if request.baseline.is_some() {
        // A disposable provision cannot fence a baseline that does not exist
        // until the gateway allocates and reports it.
        return Err(ProfileSetupError::BaselineFenceMismatch);
    }
    if operation.requires_operation_id() && request.operation_id.is_none() {
        return Err(ProfileSetupError::InvalidRequest);
    }
    if operation.is_mutation() && request.run_active {
        return Err(ProfileSetupError::ActiveRunConflict);
    }
    Ok(AdmittedProfileSetup {
        operation,
        instance_id: request.instance_id.clone(),
        profile_id: request.profile_id.clone(),
        operation_id: request.operation_id.clone(),
    })
}

/// An authoritative summary the owner reported back for a profile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProfileReadback {
    /// The profile identity the owner reported.
    pub profile_id: String,
    /// Lowercase SHA-256 digest of the profile's admitted baseline.
    pub baseline_digest: String,
    /// Whether the owner reports the profile as available for use.
    pub available: bool,
}

/// A readback the owner reported for a mutation this harness admitted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedProfileReadback {
    profile_id: String,
}

impl VerifiedProfileReadback {
    /// The verified profile identity, safe to hand to downstream setup.
    #[must_use]
    pub fn profile_id(&self) -> &str {
        &self.profile_id
    }
}

impl ProfileReadback {
    /// Verifies that this readback is the owner's answer for `admitted`.
    ///
    /// Fails closed when the identity differs, when the baseline is not a
    /// lowercase SHA-256 digest, or when the owner reports the profile as
    /// unavailable. A selection must additionally match the admitted fence, so a
    /// silent substitution reports `ReadbackMismatch` rather than progressing.
    pub fn verify(
        self,
        admitted: &AdmittedProfileSetup,
        admitted_baseline: Option<&str>,
    ) -> Result<VerifiedProfileReadback, ProfileSetupError> {
        let expected = admitted
            .profile_id()
            .ok_or(ProfileSetupError::ReadbackMismatch)?;
        if self.profile_id != expected || !self.available {
            return Err(ProfileSetupError::ReadbackMismatch);
        }
        if self.baseline_digest.len() != 64
            || !self
                .baseline_digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(ProfileSetupError::ReadbackMismatch);
        }
        if let Some(fenced) = admitted_baseline {
            if self.baseline_digest != fenced {
                return Err(ProfileSetupError::ReadbackMismatch);
            }
        }
        Ok(VerifiedProfileReadback {
            profile_id: self.profile_id,
        })
    }
}
