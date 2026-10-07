// SPDX-License-Identifier: MIT

use crate::ExoProcessConfig;
use crate::exo::{
    EXO_CONTRACT_VERSION, EXO_SOURCE_REVISION, ExoCapabilityDescriptor, ExoCapabilityState,
    ExoContextMode, ExoDecisionKind, ExoIdentity, ExoLimits, ExoPlatform, ExoPrivateStatePolicy,
    ExoProfile, ExoRestrictedProfile, ExoRuntime, ExoTrustedConfiguration,
};
use crate::exo_bridge_configuration::SyntheticLoopbackInspection;

use super::{SyntheticExoAdmissionError, SyntheticExoAdmissionPlan};

fn identity(endpoint: &str) -> ExoIdentity {
    ExoIdentity {
        source_revision: EXO_SOURCE_REVISION.to_owned(),
        package_digest: Some("a".repeat(64)),
        extension_digest: Some("b".repeat(64)),
        bridge_digest: Some("c".repeat(64)),
        model_binding: Some(String::from("o3-pro")),
        provider: Some(String::from("openai")),
        endpoint: Some(endpoint.to_owned()),
        prompt_digest: Some("d".repeat(64)),
        tool_digest: Some("e".repeat(64)),
        config_digest: Some("f".repeat(64)),
        contract_version: EXO_CONTRACT_VERSION.to_owned(),
        native_instance_id: Some(String::from("fixture-instance")),
    }
}

fn policy() -> ExoPrivateStatePolicy {
    ExoPrivateStatePolicy {
        state_root: String::from("/opt/h391-fixture/state"),
        cache_root: String::from("/opt/h391-fixture/cache"),
        temp_root: String::from("/opt/h391-fixture/temp"),
        quota_bytes: 1 << 30,
        max_retention_days: 7,
        permissions_octal: 0o700,
    }
}

fn configuration() -> Result<
    (
        ExoCapabilityDescriptor,
        ExoTrustedConfiguration,
        ExoIdentity,
    ),
    String,
> {
    let identity = identity("http://127.0.0.1:4319");
    let mut descriptor =
        ExoCapabilityDescriptor::source_review().map_err(|error| error.to_string())?;
    descriptor.identity = identity.clone();
    // These descriptor fields are deliberately not projected into the synthetic admission report.
    descriptor.lifecycle.cancellation = ExoCapabilityState::Supported;
    descriptor.lifecycle.recovery = ExoCapabilityState::Supported;
    let private_state = policy();
    let trusted = ExoTrustedConfiguration {
        identity: identity.clone(),
        platform: ExoPlatform::LinuxX86_64,
        profile: ExoProfile::Standard,
        context_mode: ExoContextMode::Fresh,
        runtime: ExoRuntime::Responses,
        limits: ExoLimits::reviewed(),
        restricted: ExoRestrictedProfile {
            tool_catalog: crate::ExoToolCatalog::reviewed(),
            state: private_state,
        },
    };
    Ok((descriptor, trusted, identity))
}

fn inspection(identity: ExoIdentity) -> Result<SyntheticLoopbackInspection, String> {
    let process = ExoProcessConfig::new(
        "/opt/h391-fixture/bridge",
        vec![
            String::from("--synthetic"),
            String::from("/opt/h391-fixture/config.json"),
            "f".repeat(64),
        ],
        None,
        Vec::new(),
    )
    .map_err(|_| String::from("test process configuration was refused"))?;
    Ok(SyntheticLoopbackInspection::structural_fixture(
        process,
        identity,
        policy(),
    ))
}

fn plan(
    descriptor: &ExoCapabilityDescriptor,
    trusted: &ExoTrustedConfiguration,
    identity: ExoIdentity,
) -> Result<SyntheticExoAdmissionPlan, SyntheticExoAdmissionError> {
    SyntheticExoAdmissionPlan::new(
        descriptor,
        trusted,
        inspection(identity).map_err(|_| SyntheticExoAdmissionError::ProfileMismatch)?,
        String::from("execution-1"),
        String::from("request-7"),
        String::from("turn-9"),
    )
}

#[test]
fn plan_binds_all_identity_axes_and_builds_only_the_concrete_process_transport()
-> Result<(), String> {
    let (descriptor, trusted, identity) = configuration()?;
    let plan = plan(&descriptor, &trusted, identity).map_err(|error| error.to_string())?;
    assert_eq!(
        plan.report.identity.endpoint.as_deref(),
        Some("http://127.0.0.1:4319")
    );
    assert_eq!(plan.report.profile, ExoProfile::Standard);
    assert_eq!(plan.report.context_mode, ExoContextMode::Fresh);
    assert_eq!(plan.report.runtime, ExoRuntime::Responses);
    assert_eq!(plan.report.platform, ExoPlatform::LinuxX86_64);
    assert_eq!(plan.report.limits, ExoLimits::reviewed());
    assert!(
        !plan
            .admission
            .decision_kinds
            .contains(&ExoDecisionKind::Recovery)
    );
    let transport = plan.into_transport();
    assert_eq!(transport.admission().identity, trusted.identity);
    // The fixture bridge path is intentionally nonexistent: plan construction and conversion do
    // not spawn it. Process launch is the later ExoTransport::exchange effect.
    Ok(())
}

#[test]
fn rejects_changed_identity_profile_limits_private_policy_and_control_ids() -> Result<(), String> {
    let (descriptor, mut trusted, original_identity) = configuration()?;
    let mut changed_identity = original_identity.clone();
    changed_identity.config_digest = Some("9".repeat(64));
    assert!(matches!(
        plan(&descriptor, &trusted, changed_identity),
        Err(SyntheticExoAdmissionError::IdentityMismatch)
    ));

    trusted.profile = ExoProfile::Map;
    assert!(matches!(
        plan(&descriptor, &trusted, original_identity.clone()),
        Err(SyntheticExoAdmissionError::ProfileMismatch)
    ));
    trusted.profile = ExoProfile::Standard;
    trusted.limits.max_turns = 2;
    assert!(matches!(
        plan(&descriptor, &trusted, original_identity.clone()),
        Err(SyntheticExoAdmissionError::ProfileMismatch)
    ));
    trusted.limits = ExoLimits::reviewed();
    trusted.restricted.state.state_root = String::from("/opt/h391-fixture/other-state");
    assert!(matches!(
        plan(&descriptor, &trusted, original_identity.clone()),
        Err(SyntheticExoAdmissionError::PrivateStateMismatch)
    ));
    trusted.restricted.state = policy();

    let invalid_process = ExoProcessConfig::new(
        "/opt/h391-fixture/bridge",
        vec![
            String::from("--synthetic"),
            String::from("/opt/config"),
            "f".repeat(64),
        ],
        None,
        Vec::new(),
    )
    .map_err(|_| String::from("test process configuration was refused"))?;
    let invalid_inspection = SyntheticLoopbackInspection::structural_fixture(
        invalid_process,
        original_identity,
        policy(),
    );
    assert!(matches!(
        SyntheticExoAdmissionPlan::new(
            &descriptor,
            &trusted,
            invalid_inspection,
            String::from("bad id"),
            String::from("request-7"),
            String::from("turn-9"),
        ),
        Err(SyntheticExoAdmissionError::Wire(_))
    ));
    Ok(())
}

#[test]
fn rejects_non_loopback_routes_and_malformed_descriptor_capabilities() -> Result<(), String> {
    let (mut descriptor, trusted, mut inspected) = configuration()?;
    inspected.endpoint = Some(String::from("https://api.openai.com/v1"));
    assert!(matches!(
        plan(&descriptor, &trusted, inspected.clone()),
        Err(SyntheticExoAdmissionError::Identity(
            crate::ExoIdentityError::InvalidEndpoint
        ))
    ));

    inspected = identity("http://127.0.0.1:4319");
    descriptor.decision_kinds.push(ExoDecisionKind::Action);
    assert!(matches!(
        plan(&descriptor, &trusted, inspected.clone()),
        Err(SyntheticExoAdmissionError::IdentityMismatch)
            | Err(SyntheticExoAdmissionError::DecisionKindsMismatch)
    ));
    descriptor.decision_kinds.pop();
    descriptor.limits.max_response_bytes = 1;
    assert!(matches!(
        plan(&descriptor, &trusted, inspected),
        Err(SyntheticExoAdmissionError::ProfileMismatch)
    ));
    Ok(())
}

#[test]
fn synthetic_route_requires_literal_loopback_nonzero_port_model_and_provider() -> Result<(), String>
{
    let (descriptor, trusted, _) = configuration()?;
    for endpoint in [
        "http://localhost:4319",
        "http://127.0.0.1:0",
        "http://127.0.0.1:+4319",
        "http://192.0.2.10:4319",
        "https://api.openai.com/v1",
    ] {
        let result = plan(&descriptor, &trusted, identity(endpoint));
        assert!(matches!(
            result,
            Err(SyntheticExoAdmissionError::Identity(
                crate::ExoIdentityError::InvalidEndpoint
            ))
        ));
    }
    let mut wrong_model = identity("http://127.0.0.1:4319");
    wrong_model.model_binding = Some(String::from("o3-mini"));
    assert!(matches!(
        plan(&descriptor, &trusted, wrong_model),
        Err(SyntheticExoAdmissionError::Identity(
            crate::ExoIdentityError::InvalidModelBinding
        ))
    ));
    let mut wrong_provider = identity("http://127.0.0.1:4319");
    wrong_provider.provider = Some(String::from("other"));
    assert!(matches!(
        plan(&descriptor, &trusted, wrong_provider),
        Err(SyntheticExoAdmissionError::IdentityMismatch)
    ));
    Ok(())
}
