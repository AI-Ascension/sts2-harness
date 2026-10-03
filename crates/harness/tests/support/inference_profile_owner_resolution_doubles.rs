// SPDX-License-Identifier: MIT

//! Shared synthetic owner doubles for the owner-authoritative
//! inference-profile resolution suite (Refs #799).
//!
//! Everything here is synthetic and labelled as such: the catalogs are in-memory
//! owner doubles and the authoring store is in-memory. No provider, model,
//! credential or native host is contacted. A pass here is component evidence that
//! the owner is the single admission authority over inference-profile
//! references. It is not provider-execution evidence, and it says nothing about
//! whether any named profile can actually reach a model.

#![allow(clippy::expect_used)]
#![allow(dead_code)]

use std::sync::Arc;

use serde_json::{Value, json};
use sts2_harness::management::{
    AuthContext, CapabilityPort, DefinitionPort, DiffResult,
    INFERENCE_PROFILE_CATALOG_SCHEMA_VERSION, INFERENCE_PROFILE_SCHEMA_VERSION,
    InferenceProfileBudgets, InferenceProfileCatalog, InferenceProfileContinuity,
    InferenceProfileDescriptor, InferenceProfileGrants, InferenceProfileState, InspectionResult,
    MANAGEMENT_SCHEMA_VERSION, ManagementError, ManagementService, STUDIO_SCHEMA_VERSION,
    StudioCreateDraftRequest, StudioPublishDraftRequest, StudioPublishResponse, ValidateRequest,
    ValidateResponse, ValidationResult, decode_strict, digest_value,
};
use sts2_harness::workflow::WorkflowDefinition;

pub(crate) const CONTEXT_REF: &str = "context.live.v1";
pub(crate) const DECISION_PROFILE: &str = "decision.synthetic.v1";
pub(crate) const PLANNER_PROFILE: &str = "planner.synthetic.v1";
/// A floating id no catalog in this suite advertises: exactly the shape the
/// Studio browser-side gate refuses and the owner used to accept.
pub(crate) const UNCATALOGUED_PROFILE: &str = "sts2.combat.planner.v1";

pub(crate) fn actor() -> Result<AuthContext, Box<dyn std::error::Error>> {
    Ok(AuthContext::new("operator", ["workflow:*".to_owned()])?)
}

/// Serves exactly the two labelled synthetic descriptors the served synthetic
/// owner advertises. Nothing else resolves.
pub(crate) fn catalog() -> Result<InferenceProfileCatalog, Box<dyn std::error::Error>> {
    let descriptors = vec![
        descriptor(DECISION_PROFILE, "decide")?,
        descriptor(PLANNER_PROFILE, "adaptive_region")?,
    ];
    let catalog = InferenceProfileCatalog {
        schema_version: INFERENCE_PROFILE_CATALOG_SCHEMA_VERSION.to_owned(),
        owner_id: "test-owner".to_owned(),
        owner_version: "1.0.0".to_owned(),
        catalog_digest: String::new(),
        descriptors,
    }
    .seal()?;
    catalog.validate()?;
    Ok(catalog)
}

pub(crate) fn descriptor(
    profile_id: &str,
    node_kind: &str,
) -> Result<InferenceProfileDescriptor, Box<dyn std::error::Error>> {
    Ok(InferenceProfileDescriptor {
        schema_version: INFERENCE_PROFILE_SCHEMA_VERSION.to_owned(),
        profile_id: profile_id.to_owned(),
        version: "1.0.0".to_owned(),
        digest: String::new(),
        adapter: "synthetic.provider.v1".to_owned(),
        requested_model: "synthetic.model.v1".to_owned(),
        resolved_model: None,
        prompt_revision: "synthetic.prompt.v1".to_owned(),
        settings_revision: "synthetic.settings.v1".to_owned(),
        supported_settings: vec![
            "max_provider_calls".to_owned(),
            "max_output_tokens".to_owned(),
        ],
        operations: vec![node_kind.to_owned()],
        node_kinds: vec![node_kind.to_owned()],
        context_compatibility: if node_kind == "decide" {
            vec![CONTEXT_REF.to_owned()]
        } else {
            Vec::new()
        },
        continuity: InferenceProfileContinuity::default(),
        effective_budgets: InferenceProfileBudgets {
            max_input_bytes: 128 * 1024,
            max_output_tokens: 4096,
            max_provider_calls: 64,
        },
        grants: InferenceProfileGrants {
            select: true,
            edit: false,
        },
        state: InferenceProfileState::Available,
    }
    .seal()?)
}

/// A two-node definition: one `decide` and one `adaptive_region`.
pub(crate) fn definition(decide_ref: &str, planner_ref: &str) -> Value {
    json!({
        "schema_version": "ascension.workflow/v1",
        "workflow_id": "fixture.owner-resolution",
        "version": "0.1.0",
        "mode": "dynamic",
        "game_profile": "sts2-live-v1",
        "policy_ref": "policy.live.v1",
        "capabilities": {"required": [], "optional": []},
        "limits": {
            "max_steps": 32,
            "max_subworkflow_depth": 3,
            "max_provider_calls": 8,
            "max_parallel_analyses": 1,
            "max_output_tokens": 4096
        },
        "entry_graph": "main",
        "graphs": [{
            "id": "main",
            "entry_node": "decide",
            "nodes": [
                {
                    "id": "decide",
                    "kind": "decide",
                    "config": {
                        "decision_profile_ref": decide_ref,
                        "context_ref": CONTEXT_REF
                    }
                },
                {
                    "id": "plan",
                    "kind": "adaptive_region",
                    "config": {
                        "region_id": "region-1",
                        "planner_profile_ref": planner_ref,
                        "allowed_operations": ["observe.fair-play.v1"],
                        "max_plan_nodes": 4,
                        "max_plan_edges": 4,
                        "max_replans": 2,
                        "output_type": "DecisionProposal"
                    }
                },
                {"id": "done", "kind": "terminal", "config": {"outcome": "completed"}}
            ],
            "edges": [
                {"from": "decide", "to": "plan", "on": "ok", "priority": 0},
                {"from": "plan", "to": "done", "on": "ok", "priority": 0}
            ]
        }]
        ,
        "annotations": {"summary": "Synthetic owner-resolution fixture.", "synthetic": true}
    })
}

/// A capability port serving `served`, so a test can model an owner with a
/// catalog and one without.
pub(crate) struct ServedCatalog(Option<InferenceProfileCatalog>);

impl CapabilityPort for ServedCatalog {
    fn capabilities(&self) -> Result<Value, ManagementError> {
        Ok(json!({"capabilities": ["workflow.live", "live.workflow.v1"]}))
    }

    fn inference_profile_catalog(
        &self,
        _actor: &AuthContext,
    ) -> Result<Option<InferenceProfileCatalog>, ManagementError> {
        Ok(self.0.clone())
    }
}

/// Compiles the submitted definition, so the service's own profile fences are
/// what a test observes rather than a stubbed verdict.
pub(crate) struct DefinitionDouble;

impl DefinitionPort for DefinitionDouble {
    fn validate(
        &self,
        definition: &Value,
        _capabilities: &Value,
    ) -> Result<ValidationResult, ManagementError> {
        let digest = digest_value(definition).map_err(ManagementError::from)?;
        let bytes = serde_json::to_vec(definition)
            .map_err(|error| ManagementError::invalid("definition_encode", error.to_string()))?;
        // Strict decoding is what makes an authored reference parse at all.
        let _: WorkflowDefinition = decode_strict(&bytes)?;
        Ok(ValidationResult {
            definition_digest: digest,
            compiler: "sts2-harness.workflow-compiler.v1".to_owned(),
            diagnostics: Vec::new(),
        })
    }

    fn inspect(&self, definition: &Value) -> Result<InspectionResult, ManagementError> {
        Ok(InspectionResult {
            definition_digest: digest_value(definition).map_err(ManagementError::from)?,
            workflow_id: None,
            workflow_version: None,
            required_capabilities: Vec::new(),
            graph_count: 0,
            node_count: 0,
        })
    }

    fn diff(
        &self,
        old_definition: &Value,
        new_definition: &Value,
    ) -> Result<DiffResult, ManagementError> {
        Ok(DiffResult {
            old_definition_digest: digest_value(old_definition).map_err(ManagementError::from)?,
            new_definition_digest: digest_value(new_definition).map_err(ManagementError::from)?,
            semantic_change: false,
            changed_paths: Vec::new(),
        })
    }
}

pub(crate) fn service(
    served: Option<InferenceProfileCatalog>,
) -> Result<ManagementService, Box<dyn std::error::Error>> {
    Ok(ManagementService::in_memory()
        .with_capability_port(Arc::new(ServedCatalog(served)))
        .with_definition_port(Arc::new(DefinitionDouble)))
}

pub(crate) fn validate(
    service: &ManagementService,
    definition: &Value,
) -> Result<ValidateResponse, ManagementError> {
    service.validate(
        &actor().expect("actor"),
        ValidateRequest {
            schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
            definition: definition.clone(),
            capabilities: json!({"capabilities": ["workflow.live"]}),
        },
    )
}

/// Publishes `definition` as a fresh draft and returns the owner's response.
pub(crate) fn publish(
    service: &ManagementService,
    definition: &Value,
) -> Result<StudioPublishResponse, ManagementError> {
    let actor = actor().expect("actor");
    let draft_id = "draft-owner-resolution";
    let client_mutation_id = "mutation-owner-resolution";
    service.studio_create_draft(
        &actor,
        StudioCreateDraftRequest {
            schema_version: STUDIO_SCHEMA_VERSION.to_owned(),
            draft_id: draft_id.to_owned(),
            definition_id: "fixture.owner-resolution".to_owned(),
            client_mutation_id: client_mutation_id.to_owned(),
            document: definition.clone(),
            layout: json!({"nodes": []}),
        },
    )?;
    let draft = service.studio_draft(&actor, draft_id)?;
    service.studio_publish_draft(
        &actor,
        draft_id,
        StudioPublishDraftRequest {
            schema_version: STUDIO_SCHEMA_VERSION.to_owned(),
            client_mutation_id: client_mutation_id.to_owned(),
            expected_revision: draft.revision,
            etag: draft.etag.clone(),
            expected_definition_digest: digest_value(definition)?,
        },
    )
}
