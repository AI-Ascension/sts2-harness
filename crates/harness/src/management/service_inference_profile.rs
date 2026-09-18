// SPDX-License-Identifier: MIT

//! Inference-profile catalog reads and the service-side admission fence.

use super::super::contract::{InferenceProfileCatalog, TargetAdmissionBinding};
use super::super::inference_profile_binding::{InferenceProfileBindingSet, resolve_definition};
use super::support::authorize;
use super::{AuthContext, ManagementError, ManagementService, RunRequest};

impl ManagementService {
    /// Returns the owner-served inference-profile catalog for `GET
    /// /v1/inference-profiles`. Reading it is `workflow:read`, carries no
    /// credential, and reaches no provider: only bounded, sealed metadata is
    /// returned, and an owner without a catalog is reported as unavailable
    /// rather than substituted.
    pub fn inference_profile_catalog(
        &self,
        actor: &AuthContext,
    ) -> Result<InferenceProfileCatalog, ManagementError> {
        authorize(actor, "workflow:read", None)?;
        let catalog = self
            .capabilities
            .inference_profile_catalog(actor)?
            .ok_or_else(|| {
                ManagementError::unavailable(
                    "inference_profile_catalog_unavailable",
                    "inference-profile discovery is not attached to this workflow owner",
                )
            })?;
        catalog.validate()?;
        Ok(catalog)
    }
}

/// Resolves every inference binding of a live submission against the
/// authoritative catalog before any reservation, lease or provider exists.
///
/// `None` means the owner serves no catalog: admission then keeps today's
/// capability-prefix check, so owners without the catalog adapter are not
/// broken. A served catalog is authoritative and fails closed.
pub(super) fn admit_inference_profiles(
    service: &ManagementService,
    actor: &AuthContext,
    request: &RunRequest,
    binding: &TargetAdmissionBinding,
) -> Result<Option<InferenceProfileBindingSet>, ManagementError> {
    let Some(catalog) = service.capabilities.inference_profile_catalog(actor)? else {
        return Ok(None);
    };
    catalog.validate()?;
    let definition = request.definition.as_ref().ok_or_else(|| {
        ManagementError::unavailable(
            "inference_profile_binding_unavailable",
            "live inference-profile admission requires an inline workflow definition",
        )
    })?;
    let parsed = super::super::workflow_ports::parse_definition(definition)?;
    resolve_definition(&catalog, &parsed, &binding.target).map(Some)
}

/// Writes the resolved provenance into the admission the run record keeps.
pub(super) fn bind_inference_provenance(
    binding: Option<TargetAdmissionBinding>,
    resolved: Option<&InferenceProfileBindingSet>,
) -> Option<TargetAdmissionBinding> {
    binding.map(|mut binding| {
        if let Some(resolved) = resolved {
            binding.target.inference_profile = Some(resolved.reference());
        }
        binding
    })
}
