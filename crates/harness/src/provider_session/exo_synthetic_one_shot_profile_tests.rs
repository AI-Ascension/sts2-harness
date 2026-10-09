// SPDX-License-Identifier: MIT

use super::{CapabilityProvenance, NativeCapabilities};
use crate::exo::{
    EXO_CONTRACT_VERSION, EXO_SOURCE_REVISION, ExoCapabilityDescriptor, ExoContextMode,
    ExoIdentity, ExoLimits, ExoPlatform, ExoPrivateStatePolicy, ExoProfile, ExoRestrictedProfile,
    ExoRuntime, ExoTrustedConfiguration,
};
use crate::exo_bridge_configuration::SyntheticLoopbackInspection;
use crate::{ExoProcessConfig, ExoTransport, SyntheticExoAdmissionPlan};

fn identity() -> ExoIdentity {
    ExoIdentity {
        source_revision: EXO_SOURCE_REVISION.to_owned(),
        package_digest: Some("a".repeat(64)),
        extension_digest: Some("b".repeat(64)),
        bridge_digest: Some("c".repeat(64)),
        model_binding: Some("o3-pro".to_owned()),
        provider: Some("openai".to_owned()),
        endpoint: Some("http://127.0.0.1:4319".to_owned()),
        prompt_digest: Some("d".repeat(64)),
        tool_digest: Some("e".repeat(64)),
        config_digest: Some("f".repeat(64)),
        contract_version: EXO_CONTRACT_VERSION.to_owned(),
        native_instance_id: Some("fixture-instance".to_owned()),
    }
}

fn private_state() -> ExoPrivateStatePolicy {
    ExoPrivateStatePolicy {
        state_root: "/opt/h391-fixture/state".to_owned(),
        cache_root: "/opt/h391-fixture/cache".to_owned(),
        temp_root: "/opt/h391-fixture/temp".to_owned(),
        quota_bytes: 1 << 30,
        max_retention_days: 7,
        permissions_octal: 0o700,
    }
}

fn inspection(identity: ExoIdentity) -> Result<SyntheticLoopbackInspection, String> {
    let process = ExoProcessConfig::new(
        "/opt/h391-fixture/bridge",
        vec![
            "--synthetic".to_owned(),
            "/opt/h391-fixture/config.json".to_owned(),
            "f".repeat(64),
        ],
        None,
        Vec::new(),
    )
    .map_err(|error| error.to_string())?;
    Ok(SyntheticLoopbackInspection::structural_fixture(
        process,
        identity,
        private_state(),
    ))
}

fn plan(identity: ExoIdentity) -> Result<SyntheticExoAdmissionPlan, String> {
    let mut descriptor =
        ExoCapabilityDescriptor::source_review().map_err(|error| error.to_string())?;
    descriptor.identity = identity.clone();
    let trusted = ExoTrustedConfiguration {
        identity: identity.clone(),
        platform: ExoPlatform::LinuxX86_64,
        profile: ExoProfile::Standard,
        context_mode: ExoContextMode::Fresh,
        runtime: ExoRuntime::Responses,
        limits: ExoLimits::reviewed(),
        restricted: ExoRestrictedProfile {
            tool_catalog: crate::ExoToolCatalog::reviewed(),
            state: private_state(),
        },
    };
    SyntheticExoAdmissionPlan::new(
        &descriptor,
        &trusted,
        inspection(identity)?,
        "execution-1".to_owned(),
        "request-7".to_owned(),
        "turn-9".to_owned(),
    )
    .map_err(|error| error.to_string())
}

fn projections(identity: ExoIdentity) -> Result<(NativeCapabilities, NativeCapabilities), String> {
    let direct =
        NativeCapabilities::reviewed_synthetic_exo_one_shot(&inspection(identity.clone())?)
            .map_err(|error| error.to_string())?;
    let plan = plan(identity)?;
    let rebuilt = NativeCapabilities::reviewed_synthetic_exo_one_shot_from_plan(&plan)
        .map_err(|error| error.to_string())?;
    Ok((direct, rebuilt))
}

#[test]
fn opaque_inspection_and_plan_share_a_fixed_schema_only_profile() -> Result<(), String> {
    let identity = identity();
    let (inspected, planned) = projections(identity.clone())?;
    let plan = plan(identity.clone())?;

    assert_eq!(plan.inspected_identity(), &identity);
    assert_eq!(inspected, planned);
    assert_eq!(inspected.profile_id, "sts2-exo-synthetic-one-shot-v1");
    assert_eq!(inspected.enabled_methods, ["turn/start"]);
    assert_eq!(inspected.provenance, CapabilityProvenance::SchemaOnly);
    assert_eq!(
        inspected.native_schema_sha256,
        crate::sha256_hex(include_bytes!(
            "../../../../protocol-artifact/exo-bridge-v1/schema.json"
        ))
    );
    assert!(inspected.validate().is_ok());
    let mut transport = plan.into_transport();
    transport
        .close()
        .map_err(|error| format!("synthetic transport close refused: {error:?}"))?;
    assert!(transport.exchange(b"request", 64, 1000).is_err());
    Ok(())
}

#[test]
fn every_mutable_identity_axis_changes_the_profile_digest() -> Result<(), String> {
    let base = identity();
    let (baseline, _) = projections(base.clone())?;
    let mut changed = Vec::new();
    for (name, slot, value) in [
        ("package", 0, "1"),
        ("extension", 1, "2"),
        ("bridge", 2, "3"),
        ("prompt", 3, "4"),
        ("tool", 4, "5"),
        ("configuration", 5, "6"),
    ] {
        let mut identity = base.clone();
        match slot {
            0 => identity.package_digest = Some(value.repeat(64)),
            1 => identity.extension_digest = Some(value.repeat(64)),
            2 => identity.bridge_digest = Some(value.repeat(64)),
            3 => identity.prompt_digest = Some(value.repeat(64)),
            4 => identity.tool_digest = Some(value.repeat(64)),
            _ => identity.config_digest = Some(value.repeat(64)),
        }
        changed.push((name, identity));
    }
    let mut endpoint = base.clone();
    endpoint.endpoint = Some("http://127.0.0.1:4320".to_owned());
    changed.push(("endpoint", endpoint));
    let mut instance = base;
    instance.native_instance_id = Some("another-instance".to_owned());
    changed.push(("native instance", instance));

    for (axis, identity) in changed {
        let (direct, rebuilt) = projections(identity)?;
        assert_eq!(direct, rebuilt, "both opaque proofs bind {axis}");
        assert_ne!(direct.profile_sha256, baseline.profile_sha256, "{axis}");
    }
    Ok(())
}

#[test]
fn fixed_or_malformed_identity_axes_and_routes_refuse_both_proof_paths() {
    let base = identity();
    let mut invalid = Vec::new();
    let mut changed = base.clone();
    changed.source_revision = "0".repeat(40);
    invalid.push(changed);
    let mut changed = base.clone();
    changed.contract_version = "other-contract".to_owned();
    invalid.push(changed);
    let mut changed = base.clone();
    changed.provider = Some("other-provider".to_owned());
    invalid.push(changed);
    let mut changed = base.clone();
    changed.model_binding = Some("other-model".to_owned());
    invalid.push(changed);
    for endpoint in [
        "http://localhost:4319",
        "http://192.0.2.1:4319",
        "http://127.0.0.1:0",
    ] {
        let mut changed = base.clone();
        changed.endpoint = Some(endpoint.to_owned());
        invalid.push(changed);
    }
    let mut changed = base.clone();
    changed.package_digest = None;
    invalid.push(changed);
    let mut changed = base;
    changed.extension_digest = Some("malformed".to_owned());
    invalid.push(changed);

    for identity in invalid {
        let inspected = inspection(identity.clone());
        assert!(inspected.is_ok());
        if let Ok(inspected) = inspected {
            assert!(NativeCapabilities::reviewed_synthetic_exo_one_shot(&inspected).is_err());
        }
        assert!(plan(identity).is_err());
    }
}
