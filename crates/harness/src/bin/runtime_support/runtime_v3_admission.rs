// SPDX-License-Identifier: MIT

//! Admission settings for the runtime-v3 Exo transport seam.
//!
//! `STS2_EXO_ADMISSION` selects the reviewed envelope path (`envelope`, the fail-closed default)
//! or the explicit raw-wire acknowledgement (`legacy`). The envelope path requires the complete
//! operator-trusted deployment identity, so a missing, malformed, unreviewed or unverified
//! deployment refuses the run while the runtime is still assembling settings, before it opens a
//! durable store, a gateway connection, an MCP session or a provider.
//!
//! The envelope path also *inspects* the one artifact it can read: the exact bytes of the bridge
//! executable it is about to launch are hashed into the inspected identity. That hash does **not**
//! yet decide admission. The pin mandates every identity axis (ADR 0031), only the bridge axis
//! carries inspected bytes at this seam, and `preflight` evaluates the axes in order and returns on
//! the first one that is unbound or mismatched — `package_digest` is evaluated first. The reviewed
//! envelope therefore always refuses today with `UnboundIdentity("package_digest")` before the bridge
//! pair is ever reached, so a swapped bridge fails only incidentally (as every deployment does), not
//! because its bytes were cross-checked against `STS2_EXO_BRIDGE_DIGEST`. Binding the remaining axes,
//! or refusing unbound pins before ordering the axes, is required before the bridge hash can decide
//! anything. See ADR 0032.

use sts2_harness::exo_admission::{
    AdmittedExoRuntimeTransport, ExoAdmissionMode, ExoAdmissionPlan, ExoInspectedArtifacts,
    ExoRuntimeAdmission,
};
use sts2_harness::{
    EXO_CONTRACT_VERSION, ExoContextMode, ExoIdentity, ExoLimits, ExoPlatform, ExoProcessConfig,
    ExoProcessTransport, ExoProfile, ExoRestrictedProfile, ExoRuntime, ExoTrustedConfiguration,
};

use super::runtime_v3_settings::{optional, required};

const ADMISSION_MODE: &str = "STS2_EXO_ADMISSION";
const DEFAULT_PRIVATE_STATE_ROOT: &str = "/var/lib/sts2-harness/exo-runtime";

pub(super) fn from_environment(
    bridge_executable: &str,
    map_context_enabled: bool,
) -> Result<ExoRuntimeAdmission, String> {
    match selected_mode(optional(ADMISSION_MODE)?.as_deref())? {
        ExoAdmissionMode::Enveloped => enveloped(bridge_executable, map_context_enabled),
        ExoAdmissionMode::Legacy => Ok(ExoRuntimeAdmission::legacy()),
    }
}

fn selected_mode(value: Option<&str>) -> Result<ExoAdmissionMode, String> {
    match value {
        None | Some("envelope") => Ok(ExoAdmissionMode::Enveloped),
        Some("legacy") => Ok(ExoAdmissionMode::Legacy),
        Some(_) => Err(format!(
            "{ADMISSION_MODE} must be exactly envelope or legacy"
        )),
    }
}

fn enveloped(
    bridge_executable: &str,
    map_context_enabled: bool,
) -> Result<ExoRuntimeAdmission, String> {
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
    let plan = ExoAdmissionPlan::inspected(
        trusted,
        &inspected_artifacts(bridge_executable)?,
        required("STS2_EXO_MODEL_EXECUTION_ID")?,
        required("STS2_EXO_REQUEST_ID")?,
        required("STS2_EXO_TURN_ID")?,
    );
    ExoRuntimeAdmission::enveloped(plan).map_err(String::from)
}

/// Inspects the artifacts the launch can bind to real bytes. The bridge executable is the only
/// artifact whose bytes this seam can read; it is read whole (bounded) and hashed into the inspected
/// identity, so that identity reflects the bytes actually about to run rather than the operator's
/// declaration. Every other axis stays unbound, and because the reviewed preflight evaluates those
/// (starting with `package_digest`) first, the envelope refuses with `UnboundIdentity("package_digest")`
/// before this hash is compared — so the bridge digest does not yet decide admission.
fn inspected_artifacts(bridge_executable: &str) -> Result<ExoInspectedArtifacts, String> {
    let bridge = ExoInspectedArtifacts::read(bridge_executable).map_err(|error| {
        format!("Exo admission cannot inspect the bridge artifact {bridge_executable}: {error}")
    })?;
    Ok(ExoInspectedArtifacts {
        bridge: Some(bridge),
        ..ExoInspectedArtifacts::default()
    })
}

/// Admits the runtime bridge for the assembled deployment. The runtime takes its transport only
/// here, so it cannot dispatch one that the reviewed preflight has not admitted.
pub(super) fn admit(
    admission: &ExoRuntimeAdmission,
    process: ExoProcessConfig,
) -> Result<AdmittedExoRuntimeTransport<ExoProcessTransport>, String> {
    admission
        .admit(ExoProcessTransport::new(process))
        .map_err(String::from)
}

fn private_state_root() -> Result<String, String> {
    Ok(optional("STS2_EXO_PRIVATE_STATE_ROOT")?
        .unwrap_or_else(|| DEFAULT_PRIVATE_STATE_ROOT.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::{ExoAdmissionMode, selected_mode};

    #[test]
    fn admission_mode_defaults_to_the_reviewed_envelope_and_rejects_unknown_values() {
        assert_eq!(selected_mode(None), Ok(ExoAdmissionMode::Enveloped));
        assert_eq!(
            selected_mode(Some("envelope")),
            Ok(ExoAdmissionMode::Enveloped)
        );
        assert_eq!(selected_mode(Some("legacy")), Ok(ExoAdmissionMode::Legacy));
        for value in ["", "Envelope", "ENVELOPE", "legacy ", "admitted", "raw"] {
            assert!(
                selected_mode(Some(value)).is_err(),
                "{value:?} must be rejected"
            );
        }
    }
}
