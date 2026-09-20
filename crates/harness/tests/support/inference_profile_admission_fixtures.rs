// SPDX-License-Identifier: MIT

//! Admission, target and binding fixtures for the inference-profile suites.
//!
//! Every value here is synthetic and labelled as such: no provider, model,
//! credential or native host is contacted, and the one target names no real
//! instance.

#![allow(clippy::expect_used)]
#![allow(dead_code)]

use serde_json::Value;
use sts2_harness::management::{
    Budget, CleanupState, Cursor, EVENT_SCHEMA_VERSION, EventClassification, EventPayload,
    EventType, ExecutionMode, GameOutcome, RunAdmission, RunEvent, RunSnapshot,
    RunTargetConfiguration, TARGET_CATALOG_SCHEMA_VERSION, TargetAdmissionBinding,
    TargetAvailability, TargetCatalogResponse, TargetDescriptor, WorkflowRunStatus, digest_value,
};

use super::inference_profile_catalog_fixtures::*;

pub(crate) fn admission(definition_digest: &str) -> RunAdmission {
    let snapshot = RunSnapshot {
        schema_version: "ascension.workflow-run/v1".to_owned(),
        workflow_run_id: "run-inference-fixture".to_owned(),
        definition_digest: definition_digest.to_owned(),
        run_revision: 1,
        status: WorkflowRunStatus::Created,
        game_outcome: GameOutcome::NotTerminal,
        cursor: Cursor {
            graph_id: "main".to_owned(),
            node_id: "decide".to_owned(),
            node_execution_id: "decide-exec".to_owned(),
        },
        pending_operation: None,
        budget: Budget::default(),
        cleanup: CleanupState::NotStarted,
        admission: None,
        execution_mode: None,
    };
    RunAdmission {
        initial_events: vec![RunEvent {
            schema_version: EVENT_SCHEMA_VERSION.to_owned(),
            workflow_run_id: snapshot.workflow_run_id.clone(),
            sequence: 1,
            run_revision: 1,
            event_type: EventType::RunStarted,
            definition_digest: definition_digest.to_owned(),
            node_execution_id: "management".to_owned(),
            payload: EventPayload {
                operation_id: None,
                classification: Some(EventClassification::Accepted),
                reason_code: "submitted".to_owned(),
            },
            integrity_digest: None,
        }],
        snapshot,
    }
}

/// The single live fake target every admission in this suite is scoped to. It
/// names no real instance and reaches no host, and it advertises
/// `inference_profiles` the way the served runtime advertises its one reviewed
/// provider-session adapter.
pub(crate) fn target_descriptor_advertising(inference_profiles: Vec<String>) -> TargetDescriptor {
    TargetDescriptor {
        instance_id: "instance-1".to_owned(),
        execution_profiles: vec!["live.workflow.v1".to_owned()],
        execution_mode: ExecutionMode::Live,
        compatibility_revision: "live.compatibility.v1".to_owned(),
        capability_revision: "live.capabilities.v1".to_owned(),
        availability: TargetAvailability::Available,
        supported_operations: vec!["workflow:control".to_owned(), "workflow:live".to_owned()],
        capabilities: vec!["workflow.live".to_owned()],
        game_profiles: vec!["sts2-live-v1".to_owned()],
        save_profiles: Vec::new(),
        inference_profiles,
    }
}

pub(crate) fn target_catalog() -> TargetCatalogResponse {
    target_catalog_advertising(&[])
}

pub(crate) fn target_catalog_advertising(inference_profiles: &[String]) -> TargetCatalogResponse {
    TargetCatalogResponse {
        schema_version: TARGET_CATALOG_SCHEMA_VERSION.to_owned(),
        catalog_revision: CATALOG_REVISION.to_owned(),
        targets: vec![target_descriptor_advertising(inference_profiles.to_vec())],
    }
}

/// The same target, carrying the consumer's target-level adapter selection.
pub(crate) fn target_selecting(selection: Option<&str>) -> RunTargetConfiguration {
    RunTargetConfiguration {
        instance_id: "instance-1".to_owned(),
        execution_profile: "live.workflow.v1".to_owned(),
        execution_mode: ExecutionMode::Live,
        workflow_revision: "0.1.0".to_owned(),
        compatibility_revision: "live.compatibility.v1".to_owned(),
        capability_revision: "live.capabilities.v1".to_owned(),
        game_profile: "sts2-live-v1".to_owned(),
        save_profile: None,
        inference_profile: selection.map(str::to_owned),
        context_capability: None,
        provider_capability: None,
    }
}

pub(crate) fn binding(definition: &Value, request_id: &str) -> TargetAdmissionBinding {
    binding_selecting(definition, request_id, None)
}

pub(crate) fn binding_selecting(
    definition: &Value,
    request_id: &str,
    selection: Option<&str>,
) -> TargetAdmissionBinding {
    TargetAdmissionBinding {
        schema_version: "ascension.workflow-admission/v1".to_owned(),
        request_id: request_id.to_owned(),
        workflow_definition_digest: digest_value(definition).expect("definition digest"),
        target: target_selecting(selection),
        descriptor_digest: target_descriptor_advertising(
            selection.map_or_else(Vec::new, |value| vec![value.to_owned()]),
        )
        .digest()
        .expect("descriptor digest"),
        catalog_revision: CATALOG_REVISION.to_owned(),
    }
}
