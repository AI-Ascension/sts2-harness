// SPDX-License-Identifier: MIT

//! Exact inference-profile resolution for workflow admission and dispatch.
//!
//! A definition's `decision_profile_ref` / `planner_profile_ref` is either a
//! floating profile id (resolves to the single available revision) or an exact
//! pin `profile_id:version:digest`.  Resolution refuses unknown ids, digest
//! mismatches, revoked/disabled/stale/unsupported revisions, unsupported node
//! kinds, denied selection, incompatible contexts and definition limits above
//! the profile's effective budgets — before any reservation, lease or provider
//! exists.  The resolved set is sealed by digest in
//! [`super::inference_profile_binding`] so the run record can carry
//! credential-free requested/resolved provenance.

use std::collections::BTreeSet;

use super::auth::AuthContext;
use super::contract::{
    InferenceProfileCatalog, InferenceProfileDescriptor, InferenceProfileState, validate_identifier,
};
use super::service::ManagementError;
use crate::workflow::{
    CapabilityId, Digest, ProviderProfile, ProviderRegistry, ProviderRegistryError, RegistryId,
    SemanticVersion, TypeError,
};

pub const INFERENCE_PROFILE_BINDINGS_SCHEMA_VERSION: &str =
    "ascension.inference-profile-bindings/v1";
/// Prefix of the provenance reference persisted in
/// `RunSnapshot.admission.target.inference_profile`.
pub const INFERENCE_PROFILE_PROVENANCE_PREFIX: &str = "inference-profiles.v1.";

/// Authoritative, actor-scoped catalog supplied by the owner that serves the
/// provider. Only bounded metadata crosses this port.
pub trait LiveInferenceProfileCatalogPort: Send + Sync {
    fn inference_profile_catalog(
        &self,
        actor: &AuthContext,
    ) -> Result<InferenceProfileCatalog, ManagementError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InferenceProfilePin {
    pub version: String,
    pub digest: String,
}

/// A decision or planner reference as authored in a workflow definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InferenceProfileRef {
    pub profile_id: String,
    pub pin: Option<InferenceProfilePin>,
}

impl InferenceProfileRef {
    /// `profile_id` floats; `profile_id:major.minor.patch:<sha256>` pins.
    pub fn parse(reference: &str) -> Result<Self, ManagementError> {
        validate_identifier("inference_profile_ref", reference)?;
        let mut parts = reference.rsplitn(3, ':');
        if let (Some(digest), Some(version), Some(profile_id)) =
            (parts.next(), parts.next(), parts.next())
            && Digest::new(digest).is_ok()
            && SemanticVersion::new(version).is_ok()
        {
            validate_identifier("inference_profile_id", profile_id)?;
            return Ok(Self {
                profile_id: profile_id.to_owned(),
                pin: Some(InferenceProfilePin {
                    version: version.to_owned(),
                    digest: digest.to_owned(),
                }),
            });
        }
        Ok(Self {
            profile_id: reference.to_owned(),
            pin: None,
        })
    }
}

impl InferenceProfileCatalog {
    /// Resolves one authored reference for one node kind to exactly one
    /// available, selectable descriptor, or refuses with a typed reason.
    pub fn resolve(
        &self,
        reference: &str,
        node_kind: &str,
    ) -> Result<&InferenceProfileDescriptor, ManagementError> {
        validate_identifier("inference_profile_node_kind", node_kind)?;
        let parsed = InferenceProfileRef::parse(reference)?;
        let candidates = self
            .descriptors
            .iter()
            .filter(|descriptor| descriptor.profile_id == parsed.profile_id)
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            return Err(unknown_profile());
        }
        let descriptor = match &parsed.pin {
            Some(pin) => resolve_pinned(&candidates, &parsed.profile_id, pin, node_kind)?,
            None => resolve_floating(&candidates, node_kind)?,
        };
        admit_state(descriptor)?;
        if !descriptor.node_kinds.iter().any(|kind| kind == node_kind) {
            return Err(node_kind_unsupported());
        }
        if !descriptor.grants.select {
            return Err(ManagementError::forbidden(
                "inference_profile_select_denied",
                "the caller is not granted selection of this inference profile",
            ));
        }
        Ok(descriptor)
    }
}

fn resolve_pinned<'catalog>(
    candidates: &[&'catalog InferenceProfileDescriptor],
    profile_id: &str,
    pin: &InferenceProfilePin,
    node_kind: &str,
) -> Result<&'catalog InferenceProfileDescriptor, ManagementError> {
    let descriptor = candidates
        .iter()
        .copied()
        .find(|descriptor| descriptor.version == pin.version)
        .ok_or_else(unknown_profile)?;
    if descriptor.digest != pin.digest {
        return Err(ManagementError::conflict(
            "inference_profile_digest_mismatch",
            "pinned inference profile digest does not match the catalog revision",
        ));
    }
    // The registry is the exact id/version/digest/capability gate; the digest
    // was compared above so a denial here can only be the node kind.
    let mut registry = ProviderRegistry::default();
    for candidate in candidates {
        registry
            .register(provider_profile(candidate)?)
            .map_err(registry_error)?;
    }
    let required = BTreeSet::from([capability(node_kind)?]);
    registry
        .resolve(
            &RegistryId::new(profile_id).map_err(type_error("inference_profile_id"))?,
            &SemanticVersion::new(pin.version.as_str())
                .map_err(type_error("inference_profile_version"))?,
            &Digest::new(pin.digest.as_str()).map_err(type_error("inference_profile_digest"))?,
            &required,
        )
        .map_err(|error| match error {
            ProviderRegistryError::Missing => unknown_profile(),
            ProviderRegistryError::CapabilityDenied => node_kind_unsupported(),
            other => registry_error(other),
        })?;
    Ok(descriptor)
}

fn resolve_floating<'catalog>(
    candidates: &[&'catalog InferenceProfileDescriptor],
    node_kind: &str,
) -> Result<&'catalog InferenceProfileDescriptor, ManagementError> {
    let supporting = candidates
        .iter()
        .copied()
        .filter(|descriptor| descriptor.node_kinds.iter().any(|kind| kind == node_kind))
        .collect::<Vec<_>>();
    let mut available = supporting
        .iter()
        .copied()
        .filter(|descriptor| descriptor.state == InferenceProfileState::Available);
    match (available.next(), available.next()) {
        (Some(descriptor), None) => Ok(descriptor),
        (Some(_), Some(_)) => Err(ManagementError::conflict(
            "inference_profile_ambiguous",
            "several available revisions match; pin profile_id:version:digest",
        )),
        // No available revision: report the newest revision's state so a
        // revoked or disabled profile is named as such, not as unknown.
        (None, _) => supporting
            .into_iter()
            .max_by(|left, right| left.version.cmp(&right.version))
            .ok_or_else(node_kind_unsupported),
    }
}

fn admit_state(descriptor: &InferenceProfileDescriptor) -> Result<(), ManagementError> {
    match descriptor.state {
        InferenceProfileState::Available => Ok(()),
        InferenceProfileState::Revoked => Err(ManagementError::forbidden(
            "inference_profile_revoked",
            "the inference profile has been revoked",
        )),
        InferenceProfileState::Disabled => Err(ManagementError::unavailable(
            "inference_profile_disabled",
            "the inference profile is disabled",
        )),
        InferenceProfileState::Stale => Err(ManagementError::conflict(
            "inference_profile_stale",
            "the inference profile revision is stale; a newer revision must be adopted",
        )),
        InferenceProfileState::Unsupported => Err(ManagementError::capability(
            "inference_profile_unsupported",
            "the inference profile's model or settings are not supported by this owner",
        )),
    }
}

fn provider_profile(
    descriptor: &InferenceProfileDescriptor,
) -> Result<ProviderProfile, ManagementError> {
    let mut capabilities = BTreeSet::new();
    for kind in descriptor.node_kinds.iter().chain(&descriptor.operations) {
        capabilities.insert(capability(kind)?);
    }
    Ok(ProviderProfile {
        id: RegistryId::new(descriptor.profile_id.as_str())
            .map_err(type_error("inference_profile_id"))?,
        version: SemanticVersion::new(descriptor.version.as_str())
            .map_err(type_error("inference_profile_version"))?,
        digest: Digest::new(descriptor.digest.as_str())
            .map_err(type_error("inference_profile_digest"))?,
        capabilities,
        max_input_bytes: usize::try_from(descriptor.effective_budgets.max_input_bytes)
            .map_err(|_| registry_error(ProviderRegistryError::InvalidLimit))?,
        max_output_tokens: descriptor.effective_budgets.max_output_tokens,
    })
}

fn capability(value: &str) -> Result<CapabilityId, ManagementError> {
    CapabilityId::new(value).map_err(type_error("inference_profile_capability"))
}

fn type_error(code: &'static str) -> impl FnOnce(TypeError) -> ManagementError {
    move |error| ManagementError::invalid(code, error.to_string())
}

fn registry_error(error: ProviderRegistryError) -> ManagementError {
    ManagementError::invalid("inference_profile_registry", error.to_string())
}

fn unknown_profile() -> ManagementError {
    ManagementError::unavailable(
        "inference_profile_unknown",
        "the inference profile catalog does not advertise this profile revision",
    )
}

fn node_kind_unsupported() -> ManagementError {
    ManagementError::capability(
        "inference_profile_node_kind_unsupported",
        "the inference profile does not serve this node kind",
    )
}
