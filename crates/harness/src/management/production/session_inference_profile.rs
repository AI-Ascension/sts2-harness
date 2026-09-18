// SPDX-License-Identifier: MIT

//! Inference-profile admission and dispatch fences for the served-live session.
//!
//! Admission resolves every binding of the admitted definition before the
//! gateway runtime, its lease or the provider exist.  Dispatch re-resolves the
//! authored reference immediately before each provider request: a revision
//! that was revoked, disabled or replaced after admission refuses the request
//! instead of silently re-binding the admitted run to another revision.

use super::*;
use crate::management::inference_profile_binding::resolve_definition;

/// Resolves the definition against the attached catalog, or returns `None`
/// when no catalog is attached to this factory.
pub(crate) fn admit_inference_profiles(
    port: Option<&dyn LiveInferenceProfileCatalogPort>,
    actor: &AuthContext,
    request: &RunRequest,
    definition: &WorkflowDefinition,
) -> Result<Option<InferenceProfileBindingSet>, ManagementError> {
    let Some(port) = port else {
        return Ok(None);
    };
    let target = request
        .admission
        .as_ref()
        .map(|admission| &admission.target)
        .ok_or_else(|| {
            ManagementError::conflict(
                "target_admission_required",
                "served inference-profile admission requires an exact target admission binding",
            )
        })?;
    let catalog = port.inference_profile_catalog(actor)?;
    catalog.validate()?;
    resolve_definition(&catalog, definition, target).map(Some)
}

impl ProductionLiveWorkflowSession {
    pub(super) fn admit_inference_profile_binding(
        &self,
        decision_profile_ref: &str,
    ) -> Result<(), ManagementError> {
        let Some(port) = self.inference_profiles.as_ref() else {
            return Ok(());
        };
        let admitted = self.admitted_profiles.as_ref().ok_or_else(|| {
            ManagementError::unavailable(
                "inference_profile_binding_unavailable",
                "live session has no admitted inference-profile bindings",
            )
        })?;
        let binding = admitted
            .bindings
            .iter()
            .find(|binding| {
                binding.node_kind == "decide" && binding.profile_ref == decision_profile_ref
            })
            .ok_or_else(|| {
                ManagementError::conflict(
                    "inference_profile_binding_unknown",
                    "the decision profile was not admitted for this run",
                )
            })?;
        let catalog = port.inference_profile_catalog(&self.actor)?;
        catalog.validate()?;
        let descriptor = catalog.resolve(decision_profile_ref, "decide")?;
        if descriptor.version != binding.version || descriptor.digest != binding.digest {
            return Err(ManagementError::conflict(
                "inference_profile_binding_changed",
                "the inference profile revision changed after admission; the admitted run keeps its bound revision and is not re-bound",
            ));
        }
        Ok(())
    }
}
