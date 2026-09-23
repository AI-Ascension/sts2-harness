// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use super::*;
use crate::management::{
    LiveWorkflowSession, MANAGEMENT_SCHEMA_VERSION, RunTargetConfiguration,
    TARGET_ADMISSION_SCHEMA_VERSION, TARGET_CATALOG_SCHEMA_VERSION, TargetCatalogResponse,
    digest_value,
};

const PROFILE: &str = "live.workflow.v1";
const INSTANCE_ID: &str = "instance-1";
const GAME_PROFILE: &str = "sts2-live-v1";
const CATALOG_REVISION: &str = "live.catalog.v1";
const COMPATIBILITY_REVISION: &str = "live.compatibility.v1";
const CAPABILITY_REVISION: &str = "live.capabilities.v1";
const WORKFLOW_REVISION: &str = "0.1.0";

fn matching_admission() -> (RunRequest, String) {
    let definition: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../conformance/workflow-v1/valid-strict.json"
    ))
    .expect("definition fixture");
    let digest = digest_value(&definition).expect("definition digest");
    let admission = TargetAdmissionBinding {
        schema_version: TARGET_ADMISSION_SCHEMA_VERSION.to_owned(),
        request_id: "request-exec-admission".to_owned(),
        workflow_definition_digest: digest.clone(),
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
        descriptor_digest: "0".repeat(64),
        catalog_revision: "live.catalog.v1".to_owned(),
    };
    let request = RunRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        request_id: "request-exec-admission".to_owned(),
        definition: Some(definition),
        artifact_id: None,
        instance_id: "instance-1".to_owned(),
        profile: "live.workflow.v1".to_owned(),
        admission: Some(admission),
    };
    (request, digest)
}

/// The `valid-strict` vector re-shaped for live admission. `matching_admission`
/// above keeps the synthetic fixture values; the catalog/descriptor fence needs
/// a genuinely admissible live pair, so the definition, the target descriptor
/// and the binding are derived from one another here.
fn live_definition() -> serde_json::Value {
    let mut value: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../conformance/workflow-v1/valid-strict.json"
    ))
    .expect("definition fixture");
    value["annotations"]["synthetic"] = serde_json::json!(false);
    value["game_profile"] = serde_json::json!(GAME_PROFILE);
    value["policy_ref"] = serde_json::json!("policy.live.v1");
    value["capabilities"]["required"][0] = serde_json::json!("observe.fair-play.live.v1");
    value
}

fn live_target_descriptor() -> TargetDescriptor {
    TargetDescriptor {
        instance_id: INSTANCE_ID.to_owned(),
        execution_profiles: vec![PROFILE.to_owned()],
        execution_mode: ExecutionMode::Live,
        compatibility_revision: COMPATIBILITY_REVISION.to_owned(),
        capability_revision: CAPABILITY_REVISION.to_owned(),
        availability: TargetAvailability::Available,
        supported_operations: vec!["workflow:control".to_owned(), "workflow:live".to_owned()],
        capabilities: vec!["observe.fair-play.live.v1".to_owned()],
        game_profiles: vec![GAME_PROFILE.to_owned()],
        save_profiles: Vec::new(),
        inference_profiles: Vec::new(),
    }
}

fn live_admission() -> (RunRequest, String, TargetAdmissionBinding) {
    let definition = live_definition();
    let digest = digest_value(&definition).expect("definition digest");
    let descriptor = live_target_descriptor();
    let admission = TargetAdmissionBinding {
        schema_version: TARGET_ADMISSION_SCHEMA_VERSION.to_owned(),
        request_id: "request-exec-admission".to_owned(),
        workflow_definition_digest: digest.clone(),
        target: RunTargetConfiguration {
            instance_id: INSTANCE_ID.to_owned(),
            execution_profile: PROFILE.to_owned(),
            execution_mode: ExecutionMode::Live,
            workflow_revision: WORKFLOW_REVISION.to_owned(),
            compatibility_revision: COMPATIBILITY_REVISION.to_owned(),
            capability_revision: CAPABILITY_REVISION.to_owned(),
            game_profile: GAME_PROFILE.to_owned(),
            save_profile: None,
            inference_profile: None,
            context_capability: None,
            provider_capability: None,
        },
        descriptor_digest: descriptor.digest().expect("descriptor digest"),
        catalog_revision: CATALOG_REVISION.to_owned(),
    };
    let request = RunRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        request_id: admission.request_id.clone(),
        definition: Some(definition),
        artifact_id: None,
        instance_id: INSTANCE_ID.to_owned(),
        profile: PROFILE.to_owned(),
        admission: Some(admission.clone()),
    };
    (request, digest, admission)
}

fn catalog(descriptor: TargetDescriptor) -> TargetCatalogResponse {
    TargetCatalogResponse {
        schema_version: TARGET_CATALOG_SCHEMA_VERSION.to_owned(),
        catalog_revision: CATALOG_REVISION.to_owned(),
        targets: vec![descriptor],
    }
}

fn actor() -> AuthContext {
    AuthContext::new("execution-boundary-actor", ["workflow:*".to_owned()]).expect("actor")
}

/// Authoritative discovery double that returns exactly the catalog it is given.
/// `open` is unreachable in these tests: both fences refuse before a session is
/// ever constructed, which is what makes the refusals effect-free.
struct CatalogFactory {
    catalog: TargetCatalogResponse,
}

impl LiveWorkflowSessionFactory for CatalogFactory {
    fn capabilities(&self) -> serde_json::Value {
        serde_json::json!({})
    }

    fn target_catalog(
        &self,
        _actor: &AuthContext,
    ) -> Result<TargetCatalogResponse, ManagementError> {
        Ok(self.catalog.clone())
    }

    fn open(
        &self,
        _request: &RunRequest,
        _actor: &AuthContext,
        _definition: &crate::workflow::WorkflowDefinition,
        _definition_digest: &str,
    ) -> Result<Box<dyn LiveWorkflowSession>, ManagementError> {
        Err(ManagementError::unavailable(
            "test_open_unused",
            "the execution-boundary fence must refuse before a session is opened",
        ))
    }
}

#[test]
fn mismatched_admission_is_rejected_at_the_execution_boundary() {
    type Tamper = fn(&mut TargetAdmissionBinding);
    let cases: [(&str, Tamper, &str); 4] = [
        (
            "instance",
            |admission| admission.target.instance_id = "instance-2".to_owned(),
            "target_instance_mismatch",
        ),
        (
            "stale_revision",
            |admission| admission.target.workflow_revision = "9.9.9".to_owned(),
            "target_admission_stale",
        ),
        (
            "definition_digest",
            |admission| admission.workflow_definition_digest = "0".repeat(64),
            "target_admission_digest_mismatch",
        ),
        (
            "request_identity",
            |admission| admission.request_id = "other-request".to_owned(),
            "target_request_mismatch",
        ),
    ];
    for (label, tamper, expected) in cases {
        let (request, digest) = matching_admission();
        let mut admission = request.admission.clone().expect("admission");
        tamper(&mut admission);
        let error = validate_live_admission(&request, &digest, &admission)
            .expect_err("mismatched admission must be rejected");
        assert_eq!(error.code, expected, "case {label}");
    }
}

/// Positive control for the execution-side fences. If this stops passing, every
/// refusal below is unfalsifiable: a fence that refuses everything proves
/// nothing about the admission it is supposed to let through.
#[test]
fn matching_live_admission_passes_both_execution_boundary_fences() {
    let (request, digest, admission) = live_admission();
    validate_live_admission(&request, &digest, &admission)
        .expect("a fully matching live admission must pass the request/definition fence");
    let factory = CatalogFactory {
        catalog: catalog(live_target_descriptor()),
    };
    validate_live_catalog(&factory, &actor(), &admission)
        .expect("a matching catalog must pass the execution-side catalog fence");
}

#[test]
fn admission_mode_and_definition_refusals_are_rejected_at_the_execution_boundary() {
    // A non-live mode on the binding is refused by `validate_live_admission`
    // even when the request still names the live profile.
    {
        let (request, digest, mut admission) = live_admission();
        admission.target.execution_mode = ExecutionMode::Synthetic;
        let error = validate_live_admission(&request, &digest, &admission)
            .expect_err("a synthetic execution mode cannot enter the live fence");
        assert_eq!(error.code, "target_mode_mismatch");
    }
    // The admitted game profile must equal the one the workflow declares.
    {
        let (request, digest, mut admission) = live_admission();
        admission.target.game_profile = "some-other-game-profile".to_owned();
        let error = validate_live_admission(&request, &digest, &admission)
            .expect_err("a game profile the workflow does not declare must be refused");
        assert_eq!(error.code, "target_game_profile_mismatch");
    }
    // Live execution without the admitted definition is unavailable, not
    // silently satisfied from a fixture.
    {
        let (mut request, digest, admission) = live_admission();
        request.definition = None;
        let error = validate_live_admission(&request, &digest, &admission)
            .expect_err("live execution requires the admitted definition");
        assert_eq!(error.code, "artifact_port_unavailable");
    }
}

/// The execution-side re-check of the served target. `service_target_validation`
/// covers the same codes at preflight; this fence runs immediately before the
/// execution port, so a target that drifted after admission must still refuse
/// here with no session, lease or provider in existence.
#[test]
fn catalog_and_descriptor_refusals_are_rejected_at_the_execution_boundary() {
    type Mutate = fn(&mut TargetDescriptor, &mut TargetAdmissionBinding);
    let cases: [(&str, Mutate, &str); 16] = [
        (
            "target_absent",
            |descriptor, _| descriptor.instance_id = "instance-2".to_owned(),
            "target_unavailable",
        ),
        (
            "availability_unavailable",
            |descriptor, _| descriptor.availability = TargetAvailability::Unavailable,
            "target_unavailable",
        ),
        (
            "availability_revoked",
            |descriptor, _| descriptor.availability = TargetAvailability::Revoked,
            "target_revoked",
        ),
        (
            "availability_expired",
            |descriptor, _| descriptor.availability = TargetAvailability::Expired,
            "target_expired",
        ),
        (
            "mode_synthetic",
            |descriptor, _| descriptor.execution_mode = ExecutionMode::Synthetic,
            "target_mode_mismatch",
        ),
        (
            "profile_missing",
            |descriptor, _| descriptor.execution_profiles = vec!["live.workflow.v2".to_owned()],
            "target_profile_unavailable",
        ),
        (
            "operation_missing",
            |descriptor, _| descriptor.supported_operations = vec!["workflow:control".to_owned()],
            "target_operation_unavailable",
        ),
        (
            "compatibility_stale",
            |descriptor, _| descriptor.compatibility_revision = "live.compatibility.v2".to_owned(),
            "target_compatibility_stale",
        ),
        (
            "capability_stale",
            |descriptor, _| descriptor.capability_revision = "live.capabilities.v2".to_owned(),
            "target_capability_stale",
        ),
        (
            "game_profile_missing",
            |descriptor, _| descriptor.game_profiles = vec!["other-game-profile".to_owned()],
            "target_game_profile_unavailable",
        ),
        (
            "save_profile_missing",
            |descriptor, binding| {
                binding.target.save_profile = Some("save.live.v1".to_owned());
                descriptor.save_profiles = Vec::new();
            },
            "target_save_profile_unavailable",
        ),
        (
            "inference_profile_missing",
            |descriptor, binding| {
                binding.target.inference_profile = Some("inference.live.v1".to_owned());
                descriptor.inference_profiles = Vec::new();
            },
            "target_inference_profile_unavailable",
        ),
        (
            "context_capability_missing",
            |_, binding| {
                binding.target.context_capability = Some("context.live.v1".to_owned());
            },
            "target_context_capability_unavailable",
        ),
        (
            "provider_capability_missing",
            |_, binding| {
                binding.target.provider_capability = Some("provider.live.v1".to_owned());
            },
            "target_provider_capability_unavailable",
        ),
        (
            "descriptor_stale",
            |_, binding| binding.descriptor_digest = "0".repeat(64),
            "target_descriptor_stale",
        ),
        (
            "catalog_stale",
            |_, binding| binding.catalog_revision = "live.catalog.v2".to_owned(),
            "target_catalog_stale",
        ),
    ];
    for (label, tamper, expected) in cases {
        let (_, _, mut admission) = live_admission();
        let mut descriptor = live_target_descriptor();
        tamper(&mut descriptor, &mut admission);
        let factory = CatalogFactory {
            catalog: catalog(descriptor),
        };
        let error = validate_live_catalog(&factory, &actor(), &admission)
            .expect_err("a drifted target must be refused at the execution boundary");
        assert_eq!(error.code, expected, "case {label}");
    }
}

/// An unsupported catalog schema is refused before any descriptor is trusted.
#[test]
fn unsupported_catalog_schema_is_rejected_at_the_execution_boundary() {
    let (_, _, admission) = live_admission();
    let factory = CatalogFactory {
        catalog: TargetCatalogResponse {
            schema_version: "ascension.workflow-targets/v999".to_owned(),
            catalog_revision: CATALOG_REVISION.to_owned(),
            targets: vec![live_target_descriptor()],
        },
    };
    let error = validate_live_catalog(&factory, &actor(), &admission)
        .expect_err("an unsupported catalog schema must be refused");
    assert_eq!(error.code, "target_catalog_schema");
}
