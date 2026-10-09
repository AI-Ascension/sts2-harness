// SPDX-License-Identifier: MIT

//! Admission settings for the runtime-v3 Exo transport seam.
//!
//! `STS2_EXO_ADMISSION` selects the reviewed envelope path (`envelope`, the fail-closed default),
//! explicit raw-wire acknowledgement (`legacy`), or the guarded synthetic-only envelope.
//! Synthetic admission stays separate from production HTTPS preflight and refuses lookup,
//! lifecycle, and live-episode composition before runtime effects.
//!
//! The envelope path also *inspects* the artifact bytes this seam can locate. Two artifacts are read
//! whole and bounded and hashed into the inspected identity: the exact bytes of the bridge executable
//! it is about to launch, and the exact bytes of the package artifact the operator locates with
//! `STS2_EXO_PACKAGE_PATH`. Both digests are computed from the bytes read at the locator and never
//! from the operator's `STS2_EXO_BRIDGE_DIGEST`/`STS2_EXO_PACKAGE_DIGEST` declarations, so the pin
//! stays independent of the observation and a swapped package is refused as
//! `IdentityMismatch("package_digest")` rather than being admitted on the declaration it is supposed
//! to cross-check.
//!
//! The exact launch configuration then supplies the independently verified extension, model route,
//! prompt, tool and configuration identities. The gateway runtime supplies its configured instance
//! identity. This uses the bridge's own loader and requires the package locator to resolve to the
//! configured executor. Binding these axes does not promote unverified lifecycle capabilities:
//! those remain a separate admission prerequisite. See ADR 0032.

use sts2_harness::exo_admission::{
    AdmittedExoRuntimeTransport, ExoAdmissionMode, ExoAdmissionPlan, ExoAdmissionRefusal,
    ExoInspectedArtifacts, ExoRuntimeAdmission,
};
use sts2_harness::exo_bridge_configuration::PrivateStateProfile;
use sts2_harness::{
    EXO_CONTRACT_VERSION, ExoContextMode, ExoIdentity, ExoLimits, ExoPlatform, ExoProcessConfig,
    ExoProcessTransport, ExoProfile, ExoRestrictedProfile, ExoRuntime, ExoTransport,
    ExoTransportError, ExoTrustedConfiguration, SyntheticExoAdmittedTransport,
};

use super::runtime_v3_settings::{optional, required};

#[path = "runtime_v3_admission_synthetic.rs"]
mod synthetic;
use synthetic::SyntheticRuntimeAdmission;

#[cfg(test)]
#[path = "runtime_v3_admission_synthetic_tests.rs"]
mod synthetic_tests;

const ADMISSION_MODE: &str = "STS2_EXO_ADMISSION";
const PACKAGE_PATH: &str = "STS2_EXO_PACKAGE_PATH";
const DEFAULT_PRIVATE_STATE_ROOT: &str = "/var/lib/sts2-harness/exo-runtime";

pub(super) fn from_environment(
    mode: RuntimeV3AdmissionMode,
    process: &ExoProcessConfig,
    map_context_enabled: bool,
    native_instance_id: &str,
    guard: RuntimeV3SyntheticGuard<'_>,
) -> Result<RuntimeV3Admission, String> {
    let RuntimeV3SyntheticGuard {
        provider_kind,
        live_episode,
        lifecycle_enabled,
        lookup_binding_enabled,
    } = guard;
    validate_synthetic_preconditions(
        mode,
        provider_kind,
        live_episode,
        lifecycle_enabled,
        lookup_binding_enabled,
    )?;
    match mode {
        RuntimeV3AdmissionMode::Enveloped => enveloped(
            process,
            map_context_enabled,
            native_instance_id,
            lifecycle_enabled,
        )
        .map(RuntimeV3Admission::Ordinary),
        RuntimeV3AdmissionMode::Legacy => {
            Ok(RuntimeV3Admission::Ordinary(ExoRuntimeAdmission::legacy()))
        }
        RuntimeV3AdmissionMode::SyntheticEnvelope => {
            synthetic::from_environment(process, native_instance_id, map_context_enabled)
                .map(|admission| RuntimeV3Admission::Synthetic(Box::new(admission)))
        }
    }
}

fn selected_mode(value: Option<&str>) -> Result<RuntimeV3AdmissionMode, String> {
    match value {
        None | Some("envelope") => Ok(RuntimeV3AdmissionMode::Enveloped),
        Some("legacy") => Ok(RuntimeV3AdmissionMode::Legacy),
        Some("synthetic-envelope") => Ok(RuntimeV3AdmissionMode::SyntheticEnvelope),
        Some(_) => Err(format!(
            "{ADMISSION_MODE} must be exactly envelope, legacy, or synthetic-envelope"
        )),
    }
}

pub(super) fn validate_synthetic_preconditions(
    mode: RuntimeV3AdmissionMode,
    provider_kind: Option<&str>,
    live_episode: bool,
    lifecycle_enabled: bool,
    lookup_binding_enabled: bool,
) -> Result<(), String> {
    if mode != RuntimeV3AdmissionMode::SyntheticEnvelope {
        return Ok(());
    }
    if provider_kind != Some("exo") {
        return Err(String::from(
            "synthetic-envelope admission requires STS2_PROVIDER_KIND=exo",
        ));
    }
    if lookup_binding_enabled {
        return Err(String::from(
            "synthetic-envelope admission is unavailable for Harness lookup binding",
        ));
    }
    if lifecycle_enabled {
        return Err(String::from(
            "synthetic-envelope admission is unavailable with lifecycle configuration",
        ));
    }
    if live_episode {
        return Err(String::from(
            "synthetic-envelope admission is unavailable for live episodes",
        ));
    }
    Ok(())
}

/// The declared admission mode, without inspecting artifacts or assembling a plan.
///
/// Live-episode admission asks whether the reviewed envelope is in force, which is a property of
/// this declaration alone. It is read through the same parser the admission itself uses, so a
/// misspelt mode cannot be refused in one place and read as the default in another.
pub(super) fn declared_mode() -> Result<RuntimeV3AdmissionMode, String> {
    selected_mode(optional(ADMISSION_MODE)?.as_deref())
}

fn enveloped(
    process: &ExoProcessConfig,
    map_context_enabled: bool,
    native_instance_id: &str,
    lifecycle_enabled: bool,
) -> Result<ExoRuntimeAdmission, String> {
    if !std::path::Path::new(process.executable()).is_absolute() {
        return Err("Exo envelope requires an absolute bridge executable path".to_owned());
    }
    let trusted = ExoTrustedConfiguration {
        identity: ExoIdentity {
            source_revision: required("STS2_EXO_REVISION")?,
            package_digest: Some(required("STS2_EXO_PACKAGE_DIGEST")?),
            extension_digest: Some(required("STS2_EXO_EXTENSION_DIGEST")?),
            bridge_digest: Some(required("STS2_EXO_BRIDGE_DIGEST")?),
            model_binding: Some(required("STS2_EXO_MODEL_BINDING")?),
            provider: Some(required("STS2_EXO_PROVIDER")?),
            endpoint: Some(required("STS2_EXO_ENDPOINT")?),
            prompt_digest: Some(required("STS2_EXO_PROMPT_DIGEST")?),
            tool_digest: Some(required("STS2_EXO_TOOL_DIGEST")?),
            config_digest: Some(required("STS2_EXO_CONFIG_DIGEST")?),
            contract_version: EXO_CONTRACT_VERSION.to_owned(),
            native_instance_id: Some(required("STS2_EXO_NATIVE_INSTANCE_ID")?),
        },
        platform: ExoPlatform::LinuxX86_64,
        profile: if map_context_enabled {
            ExoProfile::Map
        } else {
            ExoProfile::Standard
        },
        context_mode: ExoContextMode::Fresh,
        runtime: ExoRuntime::Responses,
        limits: ExoLimits::reviewed(),
        restricted: ExoRestrictedProfile::reviewed_private(private_state_root()?),
    };
    // The package locator is required, not optional: an operator that supplies no package bytes
    // cannot have them inspected, and the seam must stay fail-closed rather than admit the
    // declaration alone.
    let package_path = required(PACKAGE_PATH)?;
    let artifacts = inspected_artifacts(process.executable(), &package_path)?;
    let observed = artifacts.identity();
    // Preserve early, discriminating package/bridge refusals before parsing further deployment
    // configuration. Matching declarations alone are never used as inspected identity.
    for (axis, actual, expected) in [
        (
            "package_digest",
            observed.package_digest,
            &trusted.identity.package_digest,
        ),
        (
            "bridge_digest",
            observed.bridge_digest,
            &trusted.identity.bridge_digest,
        ),
    ] {
        if &actual != expected {
            return Err(String::from(ExoAdmissionRefusal::Preflight(
                sts2_harness::ExoPreflightError::IdentityMismatch(axis),
            )));
        }
    }
    drop(artifacts);
    let inspected = inspected_deployment(process, &package_path, native_instance_id)?;
    if let PrivateStateProfile::GuardedV2(policy) = &inspected.private_state
        && policy != &trusted.restricted.state
    {
        return Err("Exo guarded private-state policy does not match trusted profile".to_owned());
    }
    let plan = ExoAdmissionPlan::new(
        trusted,
        inspected.identity,
        required("STS2_EXO_MODEL_EXECUTION_ID")?,
        required("STS2_EXO_REQUEST_ID")?,
        required("STS2_EXO_TURN_ID")?,
    );
    if lifecycle_enabled {
        Ok(ExoRuntimeAdmission::enveloped_lifecycle(plan))
    } else {
        ExoRuntimeAdmission::enveloped(plan).map_err(String::from)
    }
}

include!("runtime_v3_admission_inspection.rs");
