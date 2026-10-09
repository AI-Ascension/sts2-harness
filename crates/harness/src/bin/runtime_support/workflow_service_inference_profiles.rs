// SPDX-License-Identifier: MIT

//! Served-live inference-profile catalog producer.
//!
//! The runtime-v3 service advertises its one compiled decision route only when
//! the same Exo admission path used by the provider factory independently
//! inspected and validated the model, prompt, and settings identities. The
//! Enveloped producer reports unavailable identities as Unsupported; explicit
//! Legacy composition leaves this catalog unattached. The effective model stays
//! `None`; no live response has been observed here.

use std::sync::Arc;
use sts2_harness::management::{
    AuthContext, InferenceProfileCatalog, LiveInferenceProfileCatalogPort, ManagementError,
    ProductionLiveWorkflowSessionFactory,
};
use sts2_harness::provider_session::NativeCapabilities;

use super::runtime_v3_admission::RuntimeV3AdmissionMode;
use super::{RuntimeConfig, runtime_v3_settings};

/// The served factory publishes the catalog only for a frozen profile-capable admission mode.
pub(super) fn attach_profile_catalog(
    factory: ProductionLiveWorkflowSessionFactory,
    mode: RuntimeV3AdmissionMode,
    catalog: Arc<dyn LiveInferenceProfileCatalogPort>,
) -> ProductionLiveWorkflowSessionFactory {
    match mode {
        RuntimeV3AdmissionMode::Enveloped | RuntimeV3AdmissionMode::SyntheticEnvelope => {
            factory.with_inference_profile_catalog(catalog)
        }
        RuntimeV3AdmissionMode::Legacy => factory,
    }
}

pub(super) struct InferenceProfileCatalogProducer {
    provider_capabilities: NativeCapabilities,
}

impl InferenceProfileCatalogProducer {
    pub(super) fn new(provider_capabilities: NativeCapabilities) -> Self {
        Self {
            provider_capabilities,
        }
    }
}

impl LiveInferenceProfileCatalogPort for InferenceProfileCatalogProducer {
    fn inference_profile_catalog(
        &self,
        actor: &AuthContext,
    ) -> Result<InferenceProfileCatalog, ManagementError> {
        if !actor.can("workflow:read") {
            return Err(ManagementError::forbidden(
                "inference_profile_scope_denied",
                "actor cannot discover inference profiles",
            ));
        }
        let config = RuntimeConfig::from_environment()
            .map_err(|error| ManagementError::unavailable("runtime_configuration", error))?;
        let settings = runtime_v3_settings::RuntimeV3Settings::from_environment(&config)
            .map_err(|error| ManagementError::unavailable("provider_configuration", error))?;
        super::profile_dispatch::catalog(&self.provider_capabilities, &config, &settings)
    }
}
