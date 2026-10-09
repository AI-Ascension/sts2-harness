// SPDX-License-Identifier: MIT

use std::path::Path;
use std::sync::Mutex;

use sts2_harness::exo_bridge_configuration::{
    SYNTHETIC_MODEL, SyntheticLoopbackInspection, synthetic_route_admitted,
};
use sts2_harness::provider_session::NativeCapabilities;
use sts2_harness::{
    EXO_CONTRACT_VERSION, EXO_SOURCE_REVISION, ExoCapabilityDescriptor, ExoContextMode,
    ExoIdentity, ExoLimits, ExoPlatform, ExoProfile, ExoRestrictedProfile, ExoRuntime,
    ExoTrustedConfiguration,
};
use sts2_harness::{ExoProcessConfig, SyntheticExoAdmissionPlan};

const PACKAGE_PATH: &str = "STS2_EXO_PACKAGE_PATH";
const DEFAULT_PRIVATE_STATE_ROOT: &str = "/var/lib/sts2-harness/exo-runtime";

pub(crate) struct SyntheticRuntimeAdmission {
    plan: Mutex<Option<SyntheticExoAdmissionPlan>>,
    trusted_identity: ExoIdentity,
    process: ExoProcessConfig,
}

pub(super) fn from_environment(
    process: &ExoProcessConfig,
    native_instance_id: &str,
    map_context_enabled: bool,
) -> Result<SyntheticRuntimeAdmission, String> {
    if map_context_enabled {
        return Err(String::from(
            "synthetic-envelope admission supports only the standard Exo profile",
        ));
    }
    let (_, process_digest) = validate_synthetic_process(process)?;
    let trusted_identity = trusted_identity(native_instance_id)?;
    if trusted_identity.config_digest.as_deref() != Some(process_digest) {
        return Err(String::from(
            "synthetic Exo config identity does not match its process arguments",
        ));
    }
    let package_path = super::required(PACKAGE_PATH)?;
    let restricted = ExoRestrictedProfile::reviewed_private(private_state_root()?);
    restricted
        .validate()
        .map_err(|_| String::from("synthetic Exo private-state profile is invalid"))?;
    let trusted = ExoTrustedConfiguration {
        identity: trusted_identity.clone(),
        platform: ExoPlatform::LinuxX86_64,
        profile: ExoProfile::Standard,
        context_mode: ExoContextMode::Fresh,
        runtime: ExoRuntime::Responses,
        limits: ExoLimits::reviewed(),
        restricted,
    };
    let mut descriptor = ExoCapabilityDescriptor::source_review()
        .map_err(|_| String::from("synthetic Exo descriptor is unavailable"))?;
    descriptor.identity = trusted_identity.clone();
    let inspection =
        SyntheticLoopbackInspection::inspect(process.clone(), package_path, native_instance_id)
            .map_err(|error| format!("synthetic Exo inspection refused: {error}"))?;
    let plan = SyntheticExoAdmissionPlan::new(
        &descriptor,
        &trusted,
        inspection,
        super::required("STS2_EXO_MODEL_EXECUTION_ID")?,
        super::required("STS2_EXO_REQUEST_ID")?,
        super::required("STS2_EXO_TURN_ID")?,
    )
    .map_err(|error| format!("synthetic Exo admission refused: {error}"))?;
    Ok(SyntheticRuntimeAdmission {
        plan: Mutex::new(Some(plan)),
        trusted_identity,
        process: process.clone(),
    })
}

impl SyntheticRuntimeAdmission {
    pub(crate) fn trusted_identity(&self) -> &ExoIdentity {
        &self.trusted_identity
    }

    pub(crate) fn inspected_identity(&self) -> Result<ExoIdentity, String> {
        self.with_plan(|plan| Ok(plan.inspected_identity().clone()))
    }

    pub(crate) fn capabilities(&self) -> Result<NativeCapabilities, String> {
        self.with_plan(|plan| {
            NativeCapabilities::reviewed_synthetic_exo_one_shot_from_plan(plan)
                .map_err(|_| String::from("synthetic Exo profile descriptor is invalid"))
        })
    }

    pub(crate) fn admit_transport(
        &self,
        process: ExoProcessConfig,
    ) -> Result<sts2_harness::SyntheticExoAdmittedTransport, String> {
        if process != self.process {
            return Err(String::from(
                "synthetic Exo process changed after guarded inspection",
            ));
        }
        let mut plan = self
            .plan
            .lock()
            .map_err(|_| String::from("synthetic Exo admission state is unavailable"))?;
        let plan = plan
            .take()
            .ok_or_else(|| String::from("synthetic Exo admission was already consumed"))?;
        Ok(plan.into_transport())
    }

    fn with_plan<T>(
        &self,
        project: impl FnOnce(&SyntheticExoAdmissionPlan) -> Result<T, String>,
    ) -> Result<T, String> {
        let plan = self
            .plan
            .lock()
            .map_err(|_| String::from("synthetic Exo admission state is unavailable"))?;
        let plan = plan
            .as_ref()
            .ok_or_else(|| String::from("synthetic Exo admission was already consumed"))?;
        project(plan)
    }
}

pub(super) fn validate_synthetic_process(
    process: &ExoProcessConfig,
) -> Result<(&str, &str), String> {
    let [mode, configuration, digest] = process.arguments() else {
        return Err(String::from(
            "synthetic Exo requires exact --synthetic config and digest arguments",
        ));
    };
    if mode != "--synthetic"
        || !Path::new(configuration).is_absolute()
        || !valid_digest(digest)
        || process.working_directory().is_some()
        || !process.inherited_environment().is_empty()
    {
        return Err(String::from(
            "synthetic Exo process configuration is outside the bounded profile",
        ));
    }
    Ok((configuration, digest))
}

fn trusted_identity(native_instance_id: &str) -> Result<ExoIdentity, String> {
    let identity = ExoIdentity {
        source_revision: super::required("STS2_EXO_REVISION")?,
        package_digest: Some(super::required("STS2_EXO_PACKAGE_DIGEST")?),
        extension_digest: Some(super::required("STS2_EXO_EXTENSION_DIGEST")?),
        bridge_digest: Some(super::required("STS2_EXO_BRIDGE_DIGEST")?),
        model_binding: Some(super::required("STS2_EXO_MODEL_BINDING")?),
        provider: Some(super::required("STS2_EXO_PROVIDER")?),
        endpoint: Some(super::required("STS2_EXO_ENDPOINT")?),
        prompt_digest: Some(super::required("STS2_EXO_PROMPT_DIGEST")?),
        tool_digest: Some(super::required("STS2_EXO_TOOL_DIGEST")?),
        config_digest: Some(super::required("STS2_EXO_CONFIG_DIGEST")?),
        contract_version: EXO_CONTRACT_VERSION.to_owned(),
        native_instance_id: Some(super::required("STS2_EXO_NATIVE_INSTANCE_ID")?),
    };
    validate_identity_pins(&identity, native_instance_id)?;
    Ok(identity)
}

pub(super) fn validate_identity_pins(
    identity: &ExoIdentity,
    native_instance_id: &str,
) -> Result<(), String> {
    let digests = [
        &identity.package_digest,
        &identity.extension_digest,
        &identity.bridge_digest,
        &identity.prompt_digest,
        &identity.tool_digest,
        &identity.config_digest,
    ];
    let endpoint = identity.endpoint.as_deref().unwrap_or_default();
    if identity.source_revision != EXO_SOURCE_REVISION
        || identity.contract_version != EXO_CONTRACT_VERSION
        || identity.provider.as_deref() != Some("openai")
        || identity.model_binding.as_deref() != Some(SYNTHETIC_MODEL)
        || !synthetic_route_admitted(endpoint, SYNTHETIC_MODEL)
        || digests
            .iter()
            .any(|digest| !digest.as_deref().is_some_and(valid_digest))
        || identity.native_instance_id.as_deref() != Some(native_instance_id)
    {
        return Err(String::from(
            "synthetic Exo identity pins do not match the fixed loopback profile",
        ));
    }
    Ok(())
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && value.bytes().any(|byte| byte != b'0')
}

fn private_state_root() -> Result<String, String> {
    Ok(super::optional("STS2_EXO_PRIVATE_STATE_ROOT")?
        .unwrap_or_else(|| DEFAULT_PRIVATE_STATE_ROOT.to_owned()))
}
