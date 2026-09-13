// SPDX-License-Identifier: MIT

use serde_json::Value;
use sts2_harness::management::{
    ExecutionMode, MANAGEMENT_SCHEMA_VERSION, RunRequest, RunTargetConfiguration,
    TARGET_ADMISSION_SCHEMA_VERSION, TARGET_CATALOG_SCHEMA_VERSION, TargetAdmissionBinding,
    TargetAvailability, TargetCatalogResponse, TargetDescriptor, digest_value,
};

use super::LIVE_CAPABILITIES;

pub(crate) fn request(id: &str, definition: Value) -> RunRequest {
    let descriptor = target_descriptor();
    let definition_digest = digest_value(&definition).expect("definition digest");
    RunRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        request_id: id.to_owned(),
        definition: Some(definition),
        artifact_id: None,
        instance_id: "instance-1".to_owned(),
        profile: "live.workflow.v1".to_owned(),
        admission: Some(TargetAdmissionBinding {
            schema_version: TARGET_ADMISSION_SCHEMA_VERSION.to_owned(),
            request_id: id.to_owned(),
            workflow_definition_digest: definition_digest,
            target: RunTargetConfiguration {
                instance_id: "instance-1".to_owned(),
                execution_profile: "live.workflow.v1".to_owned(),
                execution_mode: ExecutionMode::Live,
                workflow_revision: "0.1.0".to_owned(),
                compatibility_revision: "live.compatibility.v1".to_owned(),
                capability_revision: "live.capabilities.v1".to_owned(),
                game_profile: "sts2-live-v1".to_owned(),
                save_profile: None,
                inference_profile: None,
                context_capability: None,
                provider_capability: None,
            },
            descriptor_digest: descriptor.digest().expect("descriptor digest"),
            catalog_revision: "live.catalog.v1".to_owned(),
        }),
    }
}

pub(crate) fn target_catalog() -> TargetCatalogResponse {
    TargetCatalogResponse {
        schema_version: TARGET_CATALOG_SCHEMA_VERSION.to_owned(),
        catalog_revision: "live.catalog.v1".to_owned(),
        targets: vec![target_descriptor()],
    }
}

fn target_descriptor() -> TargetDescriptor {
    TargetDescriptor {
        instance_id: "instance-1".to_owned(),
        execution_profiles: vec!["live.workflow.v1".to_owned()],
        execution_mode: ExecutionMode::Live,
        compatibility_revision: "live.compatibility.v1".to_owned(),
        capability_revision: "live.capabilities.v1".to_owned(),
        availability: TargetAvailability::Available,
        supported_operations: vec!["workflow:control".to_owned(), "workflow:live".to_owned()],
        capabilities: LIVE_CAPABILITIES
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
        game_profiles: vec!["sts2-live-v1".to_owned()],
        save_profiles: Vec::new(),
        inference_profiles: Vec::new(),
    }
}
