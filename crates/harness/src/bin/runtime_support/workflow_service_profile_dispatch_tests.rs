// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use super::*;
use sts2_harness::exo_admission::ExoRuntimeAdmission;
use sts2_harness::management::{
    InferenceProfileBinding, InferenceProfileBindingSet, InferenceProfileCatalog,
};
use sts2_harness::provider_session::NativeCapabilities;
use sts2_harness::{
    EXO_SOURCE_REVISION, EpisodeRunnerConfig, ExoConfig, ExoProcessConfig, ExoToolCatalog,
    RecoveryController, StabilityBarrier,
};

fn runtime_config() -> RuntimeConfig {
    RuntimeConfig {
        seed_transport: None,
        gateway_address: String::from("127.0.0.1:15525"),
        gateway_token: String::from("synthetic-token"),
        mcp_binary: String::from("mcp"),
        runtime_profile: String::from("runtime-v3-gameplay"),
        instance_id: String::from("instance-1"),
        caller_id: String::from("harness"),
        session_id: String::from("gateway-session-1"),
        lease_id: String::from("lease-1"),
        lease_epoch: 1,
        episode_profile: false,
        mcp_session_id: String::from("mcp-session-1"),
        run_id: String::from("run-1"),
        episode_id: String::from("episode-1"),
        trajectory_id: String::from("trajectory-1"),
        trace_id: String::from("trace-1"),
        artifact_id: String::from("artifact-1"),
        wait_for_combat_seconds: 0,
        settlement_timeout_seconds: 30,
        map_context_enabled: false,
        recovery_environment: Vec::new(),
    }
}

pub(super) fn runtime_settings(exo: ExoConfig) -> RuntimeV3Settings {
    RuntimeV3Settings {
        runner: EpisodeRunnerConfig::new(
            1,
            StabilityBarrier::new(1, 1).expect("valid barrier"),
            RecoveryController::new(1).expect("valid recovery bound"),
            "objective",
            Vec::new(),
        )
        .expect("valid runner"),
        exo,
        process: ExoProcessConfig::new("/bin/unused-exo-bridge", Vec::new(), None, Vec::new())
            .expect("valid process configuration"),
        admission: ExoRuntimeAdmission::legacy(),
        lifecycle: None,
        lookup_agent: None,
    }
}

fn capabilities() -> NativeCapabilities {
    let mut capabilities = NativeCapabilities::fixture();
    capabilities.binding.model_revision = "model-reviewed-1".to_owned();
    capabilities
}

pub(super) fn exo_config() -> ExoConfig {
    ExoConfig::new(EXO_SOURCE_REVISION, 64 * 1024, 4 * 1024, 1_000).expect("valid Exo config")
}

fn verified_identity(capabilities: &NativeCapabilities) -> VerifiedProfileIdentity {
    VerifiedProfileIdentity {
        requested_model: capabilities.binding.model_revision.clone(),
        prompt_revision: "a".repeat(64),
        inspected_config_digest: "b".repeat(64),
    }
}

fn sealed_catalog(
    capabilities: &NativeCapabilities,
    descriptor: InferenceProfileDescriptor,
) -> InferenceProfileCatalog {
    let catalog = InferenceProfileCatalog {
        schema_version: INFERENCE_PROFILE_CATALOG_SCHEMA_VERSION.to_owned(),
        owner_id: capabilities.binding.owner.clone(),
        owner_version: capabilities.binding.owner_revision.clone(),
        catalog_digest: String::new(),
        descriptors: vec![descriptor],
    }
    .seal()
    .expect("catalog seals");
    catalog.validate().expect("catalog is valid");
    catalog
}

fn admitted_snapshot(
    catalog: &InferenceProfileCatalog,
) -> (InferenceProfileBindingSet, AdmittedInferenceProfileBinding) {
    let descriptor = catalog.descriptors[0].clone();
    let binding = InferenceProfileBinding {
        graph_id: "main".to_owned(),
        node_id: "decision-1".to_owned(),
        node_kind: "decide".to_owned(),
        profile_ref: format!(
            "{}:{}:{}",
            descriptor.profile_id, descriptor.version, descriptor.digest
        ),
        profile_id: descriptor.profile_id.clone(),
        version: descriptor.version.clone(),
        digest: descriptor.digest.clone(),
        adapter: descriptor.adapter.clone(),
        requested_model: descriptor.requested_model.clone(),
        resolved_model: descriptor.resolved_model.clone(),
    };
    let set = InferenceProfileBindingSet {
        schema_version: "ascension.inference-profile-bindings/v1".to_owned(),
        catalog_digest: catalog.catalog_digest.clone(),
        target_inference_profile: None,
        bindings: vec![binding.clone()],
        digest: "c".repeat(64),
    };
    (
        set,
        AdmittedInferenceProfileBinding {
            binding,
            descriptor,
        },
    )
}

#[test]
fn each_served_exo_setting_changes_the_sealed_profile_and_refuses_old_admission() {
    let capabilities = capabilities();
    let identity = verified_identity(&capabilities);
    let base_config = exo_config();
    let base_descriptor =
        descriptor_from_verified_identity(&capabilities, &base_config, Some(identity.clone()))
            .expect("base descriptor");
    let base_catalog = sealed_catalog(&capabilities, base_descriptor);
    let (base_set, base_snapshot) = admitted_snapshot(&base_catalog);

    let mut changed = Vec::new();
    let mut config = base_config.clone();
    config.revision.push_str("-changed");
    changed.push(("adapter revision", config, identity.clone()));

    let mut config = base_config.clone();
    config.max_request_bytes /= 2;
    changed.push(("request bound", config, identity.clone()));

    let mut config = base_config.clone();
    config.max_response_bytes *= 2;
    changed.push(("response bound", config, identity.clone()));

    let mut config = base_config.clone();
    config.timeout_millis += 1;
    changed.push(("timeout", config, identity.clone()));

    let mut config = base_config.clone();
    config.forward_visible_seed = !config.forward_visible_seed;
    changed.push(("seed forwarding", config, identity.clone()));

    let mut config = base_config.clone();
    config.tool_catalog = ExoToolCatalog {
        tools: vec!["reviewed-tool-set-change".to_owned()],
    };
    changed.push(("tool catalog", config, identity.clone()));

    let mut inspected_config_changed = identity;
    inspected_config_changed.inspected_config_digest = "d".repeat(64);
    changed.push((
        "inspected deployment config",
        base_config.clone(),
        inspected_config_changed,
    ));

    for (axis, exo, verified) in changed {
        let descriptor = descriptor_from_verified_identity(&capabilities, &exo, Some(verified))
            .expect("changed descriptor");
        assert_ne!(
            descriptor.digest, base_snapshot.descriptor.digest,
            "changing {axis} must change the sealed descriptor"
        );
        let current_catalog = sealed_catalog(&capabilities, descriptor);
        assert_ne!(
            current_catalog.catalog_digest, base_catalog.catalog_digest,
            "changing {axis} must change the served catalog identity"
        );
        let error = validate_profile_dispatch(
            &base_set,
            std::slice::from_ref(&base_snapshot),
            &current_catalog,
        )
        .expect_err("a stale admitted snapshot must refuse before provider admission");
        assert_eq!(
            error.code, "provider_profile_dispatch_unsupported",
            "changed {axis} is a fail-closed preflight refusal"
        );
    }
}

#[test]
fn unpinned_legacy_and_planner_bindings_are_not_admitted_by_the_served_factory() {
    let capabilities = capabilities();
    let config = runtime_config();
    let settings = runtime_settings(exo_config());
    let descriptor = descriptor(&capabilities, &config, &settings)
        .expect("legacy metadata remains discoverable");
    assert_eq!(descriptor.state, InferenceProfileState::Unsupported);
    assert_eq!(descriptor.prompt_revision, UNPINNED_REVISION);
    assert_eq!(descriptor.settings_revision, UNPINNED_REVISION);
    let legacy_catalog = catalog(&capabilities, &config, &settings)
        .expect("legacy catalog remains credential-free metadata");
    assert!(
        legacy_catalog
            .resolve(LIVE_DECISION_PROFILE_ID, "decide")
            .is_err()
    );

    let identity = verified_identity(&capabilities);
    let available = descriptor_from_verified_identity(&capabilities, &settings.exo, Some(identity))
        .expect("synthetic inspected settings descriptor");
    let current_catalog = sealed_catalog(&capabilities, available);
    let (mut binding_set, mut snapshot) = admitted_snapshot(&current_catalog);
    snapshot.binding.node_kind = "adaptive_region".to_owned();
    binding_set.bindings[0].node_kind = "adaptive_region".to_owned();
    assert!(validate_profile_dispatch(&binding_set, &[snapshot], &current_catalog,).is_err());
}
