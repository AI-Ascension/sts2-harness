// SPDX-License-Identifier: MIT

//! Shared harness for the proposal-only authoring-inference suite (#105).
//!
//! Everything is synthetic: an in-memory Studio draft store, an in-memory
//! authoring-operation journal, fixture catalogs and a recording provider
//! double. No provider, model, credential or native host is contacted.

#![allow(clippy::expect_used)]
#![allow(dead_code)]

#[path = "authoring_inference_support.rs"]
mod doubles;
#[path = "inference_profile_catalog_fixtures.rs"]
mod fixtures;

pub(crate) use doubles::*;
pub(crate) use fixtures::{baseline_catalog, two_node_definition};

use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde_json::{Value, json};
use sts2_harness::management::{
    AUTHORING_INFERENCE_REQUEST_SCHEMA_VERSION, AuthContext, AuthoringInferenceBase,
    AuthoringInferenceBudget, AuthoringInferenceCatalogs, AuthoringInferenceRequest,
    AuthoringInferenceRequirement, InferenceProfileCatalog, ManagementError, ManagementService,
    MemoryAuthoringInferenceJournal, MemoryAuthoringStore, MemoryWorkflowStore,
    STUDIO_SCHEMA_VERSION, StudioCreateDraftRequest, StudioDraftRecord, digest_value,
};

pub(crate) const DRAFT_ID: &str = "draft-authoring-inference";
pub(crate) const DECIDE_PROFILE: &str = "decision.synthetic.v1";

pub(crate) fn actor() -> Result<AuthContext, Box<dyn std::error::Error>> {
    Ok(AuthContext::new("operator", ["workflow:*".to_owned()])?)
}

/// An inert base document: the endpoint only digests it, never interprets it.
pub(crate) fn base_document() -> Value {
    json!({"workflow_id": "authoring-base", "version": "0.1.0"})
}

pub(crate) fn create_request(document: Value) -> StudioCreateDraftRequest {
    StudioCreateDraftRequest {
        schema_version: STUDIO_SCHEMA_VERSION.to_owned(),
        draft_id: DRAFT_ID.to_owned(),
        definition_id: "workflow-authoring".to_owned(),
        document,
        layout: json!({"nodes": [], "edges": []}),
        client_mutation_id: "mutation-create".to_owned(),
    }
}

/// The endpoints' one pinned profile reference, derived from the served catalog.
pub(crate) fn pinned_reference(
    catalog: &InferenceProfileCatalog,
) -> Result<String, ManagementError> {
    let descriptor = catalog
        .descriptors
        .iter()
        .find(|descriptor| descriptor.profile_id == DECIDE_PROFILE)
        .ok_or_else(|| {
            ManagementError::invalid(
                "fixture_missing",
                "the fixture catalog has no decide profile",
            )
        })?;
    Ok(format!(
        "{}:{}:{}",
        descriptor.profile_id, descriptor.version, descriptor.digest
    ))
}

pub(crate) fn request_for(
    draft: &StudioDraftRecord,
    catalog: &InferenceProfileCatalog,
    client_mutation_id: &str,
) -> Result<AuthoringInferenceRequest, ManagementError> {
    Ok(AuthoringInferenceRequest {
        schema_version: AUTHORING_INFERENCE_REQUEST_SCHEMA_VERSION.to_owned(),
        client_mutation_id: client_mutation_id.to_owned(),
        inference_profile_ref: pinned_reference(catalog)?,
        base: AuthoringInferenceBase {
            draft_id: draft.draft_id.clone(),
            revision: draft.revision,
            etag: draft.etag.clone(),
            definition_digest: digest_value(&draft.document)?,
        },
        catalogs: AuthoringInferenceCatalogs {
            inference_catalog_digest: catalog.catalog_digest.clone(),
            capability_manifest_digest: digest_value(&capabilities())?,
        },
        requirement: AuthoringInferenceRequirement {
            summary: "author a two-stage decision pipeline".to_owned(),
            max_stages: 4,
        },
        budget: AuthoringInferenceBudget {
            max_provider_calls: 2,
            max_output_tokens: 4096,
            max_candidate_bytes: 64 * 1024,
        },
    })
}

pub(crate) fn service_with(
    authoring: &Arc<MemoryAuthoringStore>,
    provider: &Arc<RecordingAuthoringProvider>,
    journal: &Arc<MemoryAuthoringInferenceJournal>,
    catalog: &InferenceProfileCatalog,
    execution: &Arc<RecordingExecutionPort>,
) -> ManagementService {
    ManagementService::new(Arc::new(MemoryWorkflowStore::new()))
        .with_authoring_store(authoring.clone())
        .with_capability_port(Arc::new(AuthoringCapabilityDouble {
            catalog: catalog.clone(),
        }))
        .with_definition_port(Arc::new(AuthoringDefinitionDouble))
        .with_execution_port(execution.clone())
        .with_authoring_inference_port(provider.clone())
        .with_authoring_inference_journal(journal.clone())
}

/// One assembled suite: the service under test plus the exact seams a test
/// inspects for effects.
pub(crate) struct Suite {
    pub(crate) service: ManagementService,
    pub(crate) provider: Arc<RecordingAuthoringProvider>,
    pub(crate) execution: Arc<RecordingExecutionPort>,
    pub(crate) authoring: Arc<MemoryAuthoringStore>,
    pub(crate) journal: Arc<MemoryAuthoringInferenceJournal>,
    pub(crate) catalog: InferenceProfileCatalog,
    pub(crate) draft: StudioDraftRecord,
    pub(crate) actor: AuthContext,
}

impl Suite {
    pub(crate) fn build(
        provider: Arc<RecordingAuthoringProvider>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let authoring = Arc::new(MemoryAuthoringStore::new());
        let catalog = baseline_catalog();
        let execution = RecordingExecutionPort::new();
        let journal = Arc::new(MemoryAuthoringInferenceJournal::new());
        let service = service_with(&authoring, &provider, &journal, &catalog, &execution);
        let actor = actor()?;
        let draft = service.studio_create_draft(&actor, create_request(base_document()))?;
        Ok(Self {
            service,
            provider,
            execution,
            authoring,
            journal,
            catalog,
            draft,
            actor,
        })
    }

    pub(crate) fn request(
        &self,
        client_mutation_id: &str,
    ) -> Result<AuthoringInferenceRequest, ManagementError> {
        request_for(&self.draft, &self.catalog, client_mutation_id)
    }

    /// Asserts the proposal endpoint produced no durable effect at all.
    pub(crate) fn assert_no_effect(&self) -> Result<(), Box<dyn std::error::Error>> {
        let after = self.service.studio_draft(&self.actor, DRAFT_ID)?;
        assert_eq!(after, self.draft, "the endpoint must not modify the draft");
        assert!(
            self.service
                .studio_definitions(&self.actor)?
                .definitions
                .is_empty(),
            "the endpoint must not publish a definition"
        );
        assert_eq!(
            self.execution.submissions.load(Ordering::SeqCst),
            0,
            "the endpoint must not start a run"
        );
        Ok(())
    }
}
