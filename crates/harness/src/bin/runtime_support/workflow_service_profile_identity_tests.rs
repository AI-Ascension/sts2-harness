// SPDX-License-Identifier: MIT

use super::*;
use sts2_harness::{EXO_CONTRACT_VERSION, EXO_SOURCE_REVISION};

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
        native_instance_id: Some("instance-r3".to_owned()),
    }
}

fn capabilities(identity: &ExoIdentity) -> Result<NativeCapabilities, String> {
    NativeCapabilities::reviewed_exo_one_shot(identity)
        .map_err(|_| String::from("ordinary Exo profile builder refused the fixture identity"))
}

fn verify(
    capabilities: &NativeCapabilities,
    trusted: &ExoIdentity,
    inspected: &ExoIdentity,
    settings_revision: &str,
    instance_id: &str,
) -> Result<VerifiedProfileIdentity, String> {
    verify_identity_binding(
        capabilities,
        trusted,
        inspected,
        settings_revision,
        instance_id,
    )
    .map_err(|_| String::from("identity binding refused"))
}

#[test]
fn exact_complete_trusted_and_inspected_identity_accepts_derived_profile() -> Result<(), String> {
    let identity = inspected_identity();
    let capabilities = capabilities(&identity)?;
    let verified = verify(
        &capabilities,
        &identity,
        &identity,
        EXO_SOURCE_REVISION,
        "instance-r3",
    )?;

    assert_eq!(verified.requested_model, "o3-pro");
    assert_eq!(verified.prompt_revision, "d".repeat(64));
    assert_eq!(verified.inspected_config_digest, "f".repeat(64));
    Ok(())
}

#[test]
fn trusted_identity_must_equal_the_independently_inspected_identity() -> Result<(), String> {
    let inspected = inspected_identity();
    let capabilities = capabilities(&inspected)?;
    let mut trusted = inspected.clone();
    trusted.bridge_digest = Some("9".repeat(64));

    assert!(
        verify(
            &capabilities,
            &trusted,
            &inspected,
            EXO_SOURCE_REVISION,
            "instance-r3",
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn descriptor_sealed_for_another_complete_identity_is_rejected() -> Result<(), String> {
    let original = inspected_identity();
    let capabilities = capabilities(&original)?;
    let mut current = original;
    current.config_digest = Some("9".repeat(64));

    assert!(
        verify(
            &capabilities,
            &current,
            &current,
            EXO_SOURCE_REVISION,
            "instance-r3",
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn settings_revision_and_runtime_instance_must_match_the_inspected_identity() -> Result<(), String>
{
    let identity = inspected_identity();
    let capabilities = capabilities(&identity)?;

    assert!(
        verify(
            &capabilities,
            &identity,
            &identity,
            "different-revision",
            "instance-r3",
        )
        .is_err()
    );
    assert!(
        verify(
            &capabilities,
            &identity,
            &identity,
            EXO_SOURCE_REVISION,
            "another-instance",
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn incomplete_identity_cannot_reuse_a_previously_sealed_descriptor() -> Result<(), String> {
    let complete = inspected_identity();
    let capabilities = capabilities(&complete)?;
    let mut incomplete = complete;
    incomplete.extension_digest = None;

    assert!(
        verify(
            &capabilities,
            &incomplete,
            &incomplete,
            EXO_SOURCE_REVISION,
            "instance-r3",
        )
        .is_err()
    );
    Ok(())
}
