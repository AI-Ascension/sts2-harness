// SPDX-License-Identifier: MIT

//! Served-live inference-profile catalog producer.
//!
//! The runtime-v3 workflow service serves exactly one decision profile: the
//! reviewed provider-session capability binding (adapter and declared model
//! revision) sealed together with the operator-pinned Exo prompt and
//! configuration digests.  The same pins feed the durable `config_digest` of
//! every `DecisionReference`, so a recorded decision joins to the profile
//! revision it was produced under.  The observed model identity stays `None`:
//! nothing here inspects a live provider, and an unobserved identity is never
//! inferred from the declared one.

use sts2_harness::management::{
    AuthContext, INFERENCE_PROFILE_CATALOG_SCHEMA_VERSION, INFERENCE_PROFILE_SCHEMA_VERSION,
    InferenceProfileBudgets, InferenceProfileCatalog, InferenceProfileContinuity,
    InferenceProfileDescriptor, InferenceProfileGrants, InferenceProfileState,
    LiveInferenceProfileCatalogPort, MAX_INFERENCE_OUTPUT_TOKENS, MAX_INFERENCE_PROVIDER_CALLS,
    ManagementError,
};
use sts2_harness::provider_session::NativeCapabilities;

use super::{RuntimeConfig, runtime_v3_settings};

/// The profile id the served capability manifest advertises as
/// `workflow.provider.decision.live.v1`.
const LIVE_DECISION_PROFILE_ID: &str = "decision.live.v1";
const LIVE_DECISION_PROFILE_VERSION: &str = "1.0.0";
const LIVE_CONTEXT_REF: &str = "context.live.v1";
/// Legacy admission has no operator-pinned prompt/configuration digest; the
/// revision is published as unpinned rather than fabricated.
const UNPINNED_REVISION: &str = "legacy.unpinned";

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

fn pinned_revision(name: &str) -> Result<String, ManagementError> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Ok(value),
        Ok(_) | Err(std::env::VarError::NotPresent) => Ok(UNPINNED_REVISION.to_owned()),
        Err(error) => Err(ManagementError::unavailable(
            "inference_profile_revision_unreadable",
            format!("{name} is not readable: {error}"),
        )),
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
        let binding = &self.provider_capabilities.binding;
        let descriptor = InferenceProfileDescriptor {
            schema_version: INFERENCE_PROFILE_SCHEMA_VERSION.to_owned(),
            profile_id: LIVE_DECISION_PROFILE_ID.to_owned(),
            version: LIVE_DECISION_PROFILE_VERSION.to_owned(),
            digest: String::new(),
            adapter: binding.adapter_revision.clone(),
            requested_model: binding.model_revision.clone(),
            resolved_model: None,
            prompt_revision: pinned_revision("STS2_EXO_PROMPT_DIGEST")?,
            settings_revision: pinned_revision("STS2_EXO_CONFIG_DIGEST")?,
            supported_settings: vec![
                "max_provider_calls".to_owned(),
                "max_output_tokens".to_owned(),
            ],
            operations: vec!["decide".to_owned()],
            node_kinds: vec!["decide".to_owned()],
            context_compatibility: vec![LIVE_CONTEXT_REF.to_owned()],
            continuity: InferenceProfileContinuity {
                provider_session_continuity: false,
                survives_controller_restart: false,
            },
            effective_budgets: InferenceProfileBudgets {
                max_input_bytes: u64::try_from(settings.exo.max_request_bytes).map_err(|_| {
                    ManagementError::invalid(
                        "inference_profile_budget_invalid",
                        "Exo request bound does not fit the profile budget",
                    )
                })?,
                // The Exo bridge publishes no output-token or call bound of its
                // own; the registry ceilings are published so a definition's
                // limits remain checkable, not as a measured provider limit.
                max_output_tokens: MAX_INFERENCE_OUTPUT_TOKENS,
                max_provider_calls: MAX_INFERENCE_PROVIDER_CALLS,
            },
            grants: InferenceProfileGrants {
                select: true,
                edit: false,
            },
            state: InferenceProfileState::Available,
        }
        .seal()?;
        let catalog = InferenceProfileCatalog {
            schema_version: INFERENCE_PROFILE_CATALOG_SCHEMA_VERSION.to_owned(),
            owner_id: binding.owner.clone(),
            owner_version: binding.owner_revision.clone(),
            catalog_digest: String::new(),
            descriptors: vec![descriptor],
        }
        .seal()?;
        catalog.validate()?;
        Ok(catalog)
    }
}
