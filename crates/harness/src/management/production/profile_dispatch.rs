// SPDX-License-Identifier: MIT

//! Non-wire profile snapshots passed from workflow admission into the shipped provider factory.

use crate::episode::DecisionSource;
use crate::management::contract::InferenceProfileDescriptor;
use crate::management::inference_profile_binding::{
    InferenceProfileBinding, InferenceProfileBindingSet,
};
use crate::management::service::ManagementError;

/// One admitted workflow-node binding with the full immutable descriptor it resolved against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedInferenceProfileBinding {
    pub binding: InferenceProfileBinding,
    pub descriptor: InferenceProfileDescriptor,
}

/// Exact profile catalogue and per-node snapshots sealed for one admitted workflow definition.
///
/// This is an in-process port value, not a management JSON contract. It carries no credentials,
/// prompt bytes, endpoint, or provider response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedInferenceProfileDispatch {
    binding_set: InferenceProfileBindingSet,
    bindings: Vec<AdmittedInferenceProfileBinding>,
}

impl AdmittedInferenceProfileDispatch {
    pub(crate) fn new(
        binding_set: InferenceProfileBindingSet,
        bindings: Vec<AdmittedInferenceProfileBinding>,
    ) -> Self {
        Self {
            binding_set,
            bindings,
        }
    }

    /// The exact per-node bindings and catalogue digest admitted for this run.
    #[must_use]
    pub fn binding_set(&self) -> &InferenceProfileBindingSet {
        &self.binding_set
    }

    /// Full immutable descriptor snapshots corresponding to `binding_set`.
    #[must_use]
    pub fn bindings(&self) -> &[AdmittedInferenceProfileBinding] {
        &self.bindings
    }
}

/// A provider admission prepared before the runtime is opened and consumed after its launch fence.
///
/// Implementations should retain the exact inspected provider configuration accepted during
/// preflight. `open` runs only after the served session confirms its launch observation and policy.
pub trait LiveProviderSessionAdmission: Send {
    fn open(self: Box<Self>) -> Result<Box<dyn DecisionSource + Send>, ManagementError>;
}
