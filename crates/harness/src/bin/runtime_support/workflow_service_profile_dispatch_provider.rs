// SPDX-License-Identifier: MIT

//! Served provider factory; profile admission finishes before runtime effects.

use super::super::runtime_v3_admission::RuntimeV3AdmissionMode;
use super::super::*;
use super::*;

pub(crate) struct Provider {
    pub(crate) admission_mode: RuntimeV3AdmissionMode,
    pub(crate) provider_capabilities: NativeCapabilities,
}

fn admission_mode_error() -> ManagementError {
    ManagementError::capability(
        "provider_profile_dispatch_unsupported",
        "the served provider admission mode changed after factory composition",
    )
}

pub(super) fn validate_admission_mode(
    frozen: RuntimeV3AdmissionMode,
    selected: RuntimeV3AdmissionMode,
    settings: Option<RuntimeV3AdmissionMode>,
    expected: RuntimeV3AdmissionMode,
) -> Result<(), ManagementError> {
    if frozen != expected
        || selected != expected
        || settings.is_some_and(|settings| settings != expected)
    {
        return Err(admission_mode_error());
    }
    Ok(())
}

pub(super) fn legacy_decision_source(
    settings: runtime_v3_settings::RuntimeV3Settings,
) -> Result<Box<dyn sts2_harness::DecisionSource + Send>, ManagementError> {
    if settings.admission.mode() != RuntimeV3AdmissionMode::Legacy {
        return Err(admission_mode_error());
    }
    let transport = runtime_v3_admission::admit(&settings.admission, settings.process)
        .map_err(|error| ManagementError::unavailable("provider_admission", error))?;
    Ok(Box::new(sts2_harness::ExoDecisionSource::new(
        sts2_harness::ExoSession::new(sts2_harness::ExoProvider::new(transport, settings.exo)),
    )))
}

impl LiveProviderSessionFactory for Provider {
    fn open_provider(
        &self,
        _: &RunRequest,
        _: &AuthContext,
        _: &WorkflowDefinition,
        _: &str,
    ) -> Result<Box<dyn sts2_harness::DecisionSource + Send>, ManagementError> {
        let selected_mode =
            runtime_v3_admission::declared_mode().map_err(|_| admission_mode_error())?;
        validate_admission_mode(
            self.admission_mode,
            selected_mode,
            None,
            RuntimeV3AdmissionMode::Legacy,
        )?;
        let config = RuntimeConfig::from_environment()
            .map_err(|error| ManagementError::unavailable("runtime_configuration", error))?;
        let settings = runtime_v3_settings::RuntimeV3Settings::from_environment(&config)
            .map_err(|error| ManagementError::unavailable("provider_configuration", error))?;
        validate_admission_mode(
            self.admission_mode,
            selected_mode,
            Some(settings.admission.mode()),
            RuntimeV3AdmissionMode::Legacy,
        )?;
        legacy_decision_source(settings)
    }

    fn prepare_profiled_provider(
        &self,
        request: &RunRequest,
        _actor: &AuthContext,
        _definition: &WorkflowDefinition,
        definition_digest: &str,
        authority_binding: &RuntimeAuthorityBinding,
        profiles: &sts2_harness::management::AdmittedInferenceProfileDispatch,
    ) -> Result<Box<dyn sts2_harness::management::LiveProviderSessionAdmission>, ManagementError>
    {
        let selected_mode =
            runtime_v3_admission::declared_mode().map_err(|_| admission_mode_error())?;
        if !selected_mode.supports_profiles() {
            return Err(admission_mode_error());
        }
        validate_admission_mode(self.admission_mode, selected_mode, None, selected_mode)?;
        let config = RuntimeConfig::from_environment()
            .map_err(|error| ManagementError::unavailable("runtime_configuration", error))?;
        if config.instance_id != request.instance_id {
            return Err(ManagementError::conflict(
                "runtime_instance_mismatch",
                "configured runtime differs from target",
            ));
        }
        let settings = runtime_v3_settings::RuntimeV3Settings::from_environment(&config)
            .map_err(|error| ManagementError::unavailable("provider_configuration", error))?;
        validate_admission_mode(
            self.admission_mode,
            selected_mode,
            Some(settings.admission.mode()),
            selected_mode,
        )?;
        prepare_provider(
            &self.provider_capabilities,
            &config,
            settings,
            request,
            definition_digest,
            authority_binding,
            profiles,
        )
    }
}
