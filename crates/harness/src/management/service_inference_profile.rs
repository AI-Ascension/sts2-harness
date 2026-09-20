// SPDX-License-Identifier: MIT

//! Inference-profile catalog reads and the service-side admission fence.

use super::super::contract::{InferenceProfileCatalog, TargetAdmissionBinding};
use super::super::inference_profile_binding::{InferenceProfileBindingSet, resolve_definition};
use super::super::inference_profile_revision::{
    InferenceProfileRevisionRequest, InferenceProfileRevisionResponse, RevisionAppendOutcome,
    derive_inference_profile_revision,
};
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

    /// Adopts one new inference-profile revision through the CAS journal for
    /// `POST /v1/inference-profiles/{profile_id}/revisions`.
    ///
    /// Two independent authorities are required. The caller must hold
    /// `workflow:content:write`, the owner configuration-write scope, so the
    /// read and select grants that authorize discovery confer **no** edit
    /// authority here. The served revision must also publish `grants.edit`,
    /// so an owner can keep a profile discoverable and selectable while
    /// refusing every edit to it.
    ///
    /// The accepted edit is appended as a *new* immutable revision. The
    /// revision named by `expected_revision_digest` is never rewritten, and the
    /// compare-and-swap is performed by the journal's atomic append, so a
    /// concurrent edit loses the swap instead of silently overwriting the
    /// winner. Nothing here reads or writes a run record: an already-admitted
    /// run keeps the exact id/version/digest it resolved at admission.
    pub fn adopt_inference_profile_revision(
        &self,
        actor: &AuthContext,
        profile_id: &str,
        request: InferenceProfileRevisionRequest,
    ) -> Result<InferenceProfileRevisionResponse, ManagementError> {
        authorize(actor, "workflow:content:write", None)?;
        super::super::contract::validate_identifier("profile_id", profile_id)?;
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
        // Editing is anchored on the revision the owner *serves*: a profile the
        // catalog does not advertise cannot be edited, and the expected digest
        // must name a revision actually published for that profile.
        let served = catalog
            .descriptors
            .iter()
            .find(|descriptor| descriptor.profile_id == profile_id)
            .ok_or_else(|| {
                ManagementError::unavailable(
                    "inference_profile_unknown",
                    "the inference profile catalog does not advertise this profile",
                )
            })?;
        if served.digest != request.expected_revision_digest {
            return Err(ManagementError::conflict(
                "inference_profile_revision_conflict",
                "the expected revision digest does not name the served revision",
            ));
        }
        let candidate = derive_inference_profile_revision(served, &request)?;
        let outcome = self.journal.append(
            profile_id,
            served,
            &request.expected_revision_digest,
            &request.client_mutation_id,
            &candidate,
        )?;
        let (outcome, revision) = match outcome {
            RevisionAppendOutcome::Adopted(revision) => ("adopted", *revision),
            RevisionAppendOutcome::Replayed(revision) => ("replayed", *revision),
            RevisionAppendOutcome::Conflict(revision) => ("conflict", *revision),
        };
        Ok(InferenceProfileRevisionResponse::new(
            outcome, profile_id, revision,
        ))
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
    // This is the admission fence, and the target still carries the consumer's
    // selection, so it is passed through unchanged and checked against every
    // resolved adapter. A selection that happens to look like a recorded
    // reference is checked as a selection rather than dropped.
    resolve_definition(
        &catalog,
        &parsed,
        binding.target.inference_profile.as_deref(),
    )
    .map(Some)
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
