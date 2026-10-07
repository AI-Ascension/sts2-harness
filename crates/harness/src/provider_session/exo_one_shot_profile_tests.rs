// SPDX-License-Identifier: MIT

use super::*;
use crate::exo::{EXO_CONTRACT_VERSION, EXO_SOURCE_REVISION};

fn inspected_identity() -> ExoIdentity {
    ExoIdentity {
        source_revision: EXO_SOURCE_REVISION.to_owned(),
        package_digest: Some("a".repeat(64)),
        extension_digest: Some("b".repeat(64)),
        bridge_digest: Some("c".repeat(64)),
        model_binding: Some("o3-pro".to_owned()),
        provider: Some("openai".to_owned()),
        endpoint: Some("https://api.openai.com/v1/responses".to_owned()),
        prompt_digest: Some("d".repeat(64)),
        tool_digest: Some("e".repeat(64)),
        config_digest: Some("f".repeat(64)),
        contract_version: EXO_CONTRACT_VERSION.to_owned(),
        native_instance_id: Some("r3-instance-01".to_owned()),
    }
}

fn build(identity: &ExoIdentity) -> Result<NativeCapabilities, SessionError> {
    NativeCapabilities::reviewed_exo_one_shot(identity)
}

#[test]
fn ordinary_descriptor_binds_the_complete_inspected_identity_and_v1_schema()
-> Result<(), SessionError> {
    let identity = inspected_identity();
    let capabilities = build(&identity)?;

    assert!(capabilities.validate().is_ok());
    assert_eq!(capabilities.profile_id, EXO_SOURCE_REVISION);
    assert_eq!(capabilities.binding.adapter_revision, EXO_SOURCE_REVISION);
    assert_eq!(capabilities.native_version, "o3-pro");
    assert_eq!(capabilities.binding.model_revision, "o3-pro");
    assert_eq!(
        capabilities.native_binary_sha256,
        identity.package_digest.as_deref().unwrap_or_default()
    );
    assert_eq!(
        crate::sha256_hex(EXO_BRIDGE_SCHEMA_BYTES),
        "9120cf874af6d111c5979c2d5071749e1afb30a63c721f670a76c33b844f4b85"
    );
    assert_eq!(
        capabilities.native_schema_sha256,
        "9120cf874af6d111c5979c2d5071749e1afb30a63c721f670a76c33b844f4b85"
    );
    assert_eq!(capabilities.provenance, CapabilityProvenance::SchemaOnly);
    assert_eq!(
        capabilities.enabled_methods,
        vec![String::from("turn/start")]
    );
    assert_eq!(
        capabilities.profile_sha256,
        capabilities.binding.adapter_revision_sha256
    );
    assert!(!capabilities.strict_executable);
    assert!(!capabilities.hardening.encrypted_state);
    Ok(())
}

#[test]
fn versioned_profile_digest_changes_with_each_valid_identity_axis() -> Result<(), SessionError> {
    let identity = inspected_identity();
    let baseline = build(&identity)?;
    let updates: [fn(&mut ExoIdentity); 9] = [
        |value| value.package_digest = Some("1".repeat(64)),
        |value| value.extension_digest = Some("2".repeat(64)),
        |value| value.bridge_digest = Some("3".repeat(64)),
        |value| value.model_binding = Some("o1-pro".to_owned()),
        |value| value.endpoint = Some("https://api.openai.com/v2/responses".to_owned()),
        |value| value.prompt_digest = Some("4".repeat(64)),
        |value| value.tool_digest = Some("5".repeat(64)),
        |value| value.config_digest = Some("6".repeat(64)),
        |value| value.native_instance_id = Some("r3-instance-02".to_owned()),
    ];

    for update in updates {
        let mut changed = identity.clone();
        update(&mut changed);
        let descriptor = build(&changed)?;
        assert_ne!(baseline.profile_sha256, descriptor.profile_sha256);
    }
    Ok(())
}

#[test]
fn builder_refuses_incomplete_unreviewed_or_nonproduction_identity() {
    let mut missing = inspected_identity();
    missing.config_digest = None;
    assert!(NativeCapabilities::reviewed_exo_one_shot(&missing).is_err());

    let mut wrong_source = inspected_identity();
    wrong_source.source_revision = "1".repeat(40);
    assert!(NativeCapabilities::reviewed_exo_one_shot(&wrong_source).is_err());

    let mut wrong_contract = inspected_identity();
    wrong_contract.contract_version.push_str("-future");
    assert!(NativeCapabilities::reviewed_exo_one_shot(&wrong_contract).is_err());

    let mut wrong_provider = inspected_identity();
    wrong_provider.provider = Some("other-provider".to_owned());
    assert!(NativeCapabilities::reviewed_exo_one_shot(&wrong_provider).is_err());

    let mut wrong_endpoint = inspected_identity();
    wrong_endpoint.endpoint = Some("http://127.0.0.1:9000".to_owned());
    assert!(NativeCapabilities::reviewed_exo_one_shot(&wrong_endpoint).is_err());

    let mut unsupported_model = inspected_identity();
    unsupported_model.model_binding = Some("gpt-4.1".to_owned());
    assert!(NativeCapabilities::reviewed_exo_one_shot(&unsupported_model).is_err());
}

#[test]
fn identical_inspected_identity_produces_identical_digest() -> Result<(), SessionError> {
    let identity = inspected_identity();
    let first = build(&identity)?;
    let second = build(&identity)?;
    assert_eq!(first.profile_sha256, second.profile_sha256);
    assert_eq!(
        first.binding.descriptor_sha256,
        second.binding.descriptor_sha256
    );
    Ok(())
}
