// SPDX-License-Identifier: MIT

use super::super::super::contract_seed_v2::SeedModeV2;
use super::super::json::decode_strict;
use super::super::target_admission::{
    ExecutionMode, TARGET_CATALOG_SCHEMA_VERSION, TargetAvailability, TargetCatalogResponse,
    TargetDescriptor,
};
use super::{
    TARGET_SEED_SUPPORT_CATALOG_V2_SCHEMA, TargetSeedSupportCatalogV2, TargetSeedSupportV2,
};

const DESCRIPTOR_DIGEST: &str = "93283a6890aae5b87aa656e5df60ca7ca090114300caca6fb667d309275acfe3";
const SUPPORT_BODY: &str = r#"{"schema_version":"ascension.workflow-targets/v2","catalog_revision":"runtime-v3:profile-a","support_revision":"seed-support.v1","targets":[{"target":{"instance_id":"owner-a","execution_profiles":["live.workflow.v1"],"execution_mode":"live","compatibility_revision":"compat.v1","capability_revision":"caps.v1","availability":"available","supported_operations":["workflow:live"],"capabilities":["workflow:context:read"],"game_profiles":["sts2-live-v1"],"save_profiles":[],"inference_profiles":["exo.runtime-v3"]},"descriptor_digest":"93283a6890aae5b87aa656e5df60ca7ca090114300caca6fb667d309275acfe3","durable_candidate_binding_modes":["explicit","derive_once"],"ready_candidate_modes":["explicit"],"supported_launch_setups":[]}]}"#;

fn descriptor() -> TargetDescriptor {
    TargetDescriptor {
        instance_id: "owner-a".to_owned(),
        execution_profiles: vec!["live.workflow.v1".to_owned()],
        execution_mode: ExecutionMode::Live,
        compatibility_revision: "compat.v1".to_owned(),
        capability_revision: "caps.v1".to_owned(),
        availability: TargetAvailability::Available,
        supported_operations: vec!["workflow:live".to_owned()],
        capabilities: vec!["workflow:context:read".to_owned()],
        game_profiles: vec!["sts2-live-v1".to_owned()],
        save_profiles: Vec::new(),
        inference_profiles: vec!["exo.runtime-v3".to_owned()],
    }
}

fn support() -> TargetSeedSupportCatalogV2 {
    TargetSeedSupportCatalogV2 {
        schema_version: TARGET_SEED_SUPPORT_CATALOG_V2_SCHEMA.to_owned(),
        catalog_revision: "runtime-v3:profile-a".to_owned(),
        support_revision: "seed-support.v1".to_owned(),
        targets: vec![TargetSeedSupportV2 {
            target: descriptor(),
            descriptor_digest: DESCRIPTOR_DIGEST.to_owned(),
            durable_candidate_binding_modes: vec![SeedModeV2::Explicit, SeedModeV2::DeriveOnce],
            ready_candidate_modes: vec![SeedModeV2::Explicit],
            supported_launch_setups: Vec::new(),
        }],
    }
}

#[test]
fn v2_catalog_has_golden_body_and_exact_v1_descriptor_digest() {
    let value = support();
    let encoded = serde_json::to_string(&value);
    assert_eq!(encoded.ok().as_deref(), Some(SUPPORT_BODY));
    assert!(value.validate().is_ok());
    assert_eq!(
        descriptor().digest().ok().as_deref(),
        Some(DESCRIPTOR_DIGEST)
    );
}

#[test]
fn support_catalog_requires_exact_v1_descriptor_coverage_and_revision() {
    let value = support();
    let catalog = TargetCatalogResponse {
        schema_version: TARGET_CATALOG_SCHEMA_VERSION.to_owned(),
        catalog_revision: "runtime-v3:profile-a".to_owned(),
        targets: vec![descriptor()],
    };
    assert!(value.validate_against(&catalog).is_ok());

    let mut wrong_revision = support();
    wrong_revision.support_revision = "seed-support.v2".to_owned();
    assert!(wrong_revision.validate_against(&catalog).is_ok());
    wrong_revision.catalog_revision = "runtime-v3:other".to_owned();
    assert!(wrong_revision.validate_against(&catalog).is_err());

    let mut changed_target = catalog;
    changed_target.targets[0].capability_revision = "caps.v2".to_owned();
    assert!(value.validate_against(&changed_target).is_err());
}

#[test]
fn v1_catalog_serialization_remains_independent_of_v2_support_fields() {
    let catalog = TargetCatalogResponse {
        schema_version: TARGET_CATALOG_SCHEMA_VERSION.to_owned(),
        catalog_revision: "runtime-v3:profile-a".to_owned(),
        targets: vec![descriptor()],
    };
    let encoded = serde_json::to_string(&catalog);
    assert!(encoded.is_ok());
    let body = encoded.unwrap_or_default();
    assert!(body.starts_with("{\"schema_version\":\"ascension.workflow-targets/v1\",\"catalog_revision\":\"runtime-v3:profile-a\",\"targets\":["));
    assert!(!body.contains("support_revision"));
    assert!(!body.contains("durable_candidate_binding_modes"));
}

#[test]
fn strict_decoder_rejects_duplicate_and_unknown_fields_at_each_object_level() {
    let duplicate_top = SUPPORT_BODY.replace(
        "\"support_revision\":\"seed-support.v1\",",
        "\"support_revision\":\"seed-support.v1\",\"support_revision\":\"seed-support.v1\",",
    );
    let duplicate_nested = SUPPORT_BODY.replace(
        "\"instance_id\":\"owner-a\"",
        "\"instance_id\":\"owner-a\",\"instance_id\":\"owner-a\"",
    );
    let unknown_top = SUPPORT_BODY.replace(
        "\"support_revision\":\"seed-support.v1\",",
        "\"support_revision\":\"seed-support.v1\",\"future_field\":true,",
    );
    let unknown_target = SUPPORT_BODY.replace(
        "\"instance_id\":\"owner-a\"",
        "\"future_target_field\":true,\"instance_id\":\"owner-a\"",
    );
    for body in [duplicate_top, duplicate_nested, unknown_top, unknown_target] {
        assert!(decode_strict::<TargetSeedSupportCatalogV2>(body.as_bytes()).is_err());
    }
}

#[test]
fn strict_decoder_rejects_unknown_schema_mode_and_nonempty_launch_setup() {
    let wrong_schema = SUPPORT_BODY.replace(
        TARGET_SEED_SUPPORT_CATALOG_V2_SCHEMA,
        "ascension.workflow-targets/v9",
    );
    let unknown_mode = SUPPORT_BODY.replace(
        "\"ready_candidate_modes\":[\"explicit\"]",
        "\"ready_candidate_modes\":[\"future_mode\"]",
    );
    let launch_setup = SUPPORT_BODY.replace(
        "\"supported_launch_setups\":[]",
        "\"supported_launch_setups\":[{}]",
    );
    let decoded_schema = decode_strict::<TargetSeedSupportCatalogV2>(wrong_schema.as_bytes());
    assert!(decoded_schema.is_ok_and(|catalog| catalog.validate().is_err()));
    assert!(decode_strict::<TargetSeedSupportCatalogV2>(unknown_mode.as_bytes()).is_err());
    assert!(decode_strict::<TargetSeedSupportCatalogV2>(launch_setup.as_bytes()).is_err());
}

#[test]
fn validation_rejects_digest_duplicates_ready_without_durable_support_and_overflow() {
    let mut bad_digest = support();
    bad_digest.targets[0].descriptor_digest = "00".repeat(32);
    assert!(bad_digest.validate().is_err());

    let mut duplicate_mode = support();
    duplicate_mode.targets[0].durable_candidate_binding_modes =
        vec![SeedModeV2::Explicit, SeedModeV2::Explicit];
    assert!(duplicate_mode.validate().is_err());

    let mut unbacked_ready = support();
    unbacked_ready.targets[0].durable_candidate_binding_modes = vec![SeedModeV2::Explicit];
    unbacked_ready.targets[0].ready_candidate_modes = vec![SeedModeV2::DeriveOnce];
    assert!(unbacked_ready.validate().is_err());

    let mut too_many_targets = support();
    too_many_targets.targets = vec![too_many_targets.targets[0].clone(); 33];
    assert!(too_many_targets.validate().is_err());

    let mut duplicate_targets = support();
    duplicate_targets
        .targets
        .push(duplicate_targets.targets[0].clone());
    assert!(duplicate_targets.validate().is_err());
}

#[test]
fn support_modes_may_be_empty_without_claiming_launch_readiness() {
    let mut value = support();
    value.targets[0].durable_candidate_binding_modes.clear();
    value.targets[0].ready_candidate_modes.clear();
    assert!(value.validate().is_ok());
    assert!(value.targets[0].supported_launch_setups.is_empty());
}
