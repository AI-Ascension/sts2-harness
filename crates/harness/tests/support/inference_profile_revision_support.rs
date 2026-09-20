// SPDX-License-Identifier: MIT

//! Shared helpers for the inference-profile revision-edit suites.
//!
//! Everything here is in-memory and labelled synthetic: no provider, model,
//! credential or native host is contacted.

#![allow(clippy::expect_used)]
#![allow(dead_code)]

use std::sync::Arc;

use serde_json::Value;
use sts2_harness::management::{
    AuthContext, INFERENCE_PROFILE_REVISION_SCHEMA_VERSION, InferenceProfileBudgets,
    InferenceProfileDescriptor, InferenceProfileRevisionRequest, InferenceProfileRevisionResponse,
    MemoryInferenceProfileRevisionJournal, derive_inference_profile_revision,
};
use sts2_harness::workflow::WorkflowDefinition;

use super::inference_profile_catalog_doubles::{CatalogCapabilityDouble, Fixture};
use super::inference_profile_catalog_fixtures::{editable_catalog, two_node_definition};

pub(crate) const DECIDE: &str = "decision.synthetic.v1";
pub(crate) const PLANNER: &str = "planner.synthetic.v1";

pub(crate) fn actor(scopes: &[&str]) -> Result<AuthContext, Box<dyn std::error::Error>> {
    Ok(AuthContext::new(
        "integration.tester",
        scopes.iter().map(|scope| (*scope).to_owned()),
    )?)
}

/// The owner-configuration write scope. `workflow:read` and
/// `workflow:control` deliberately do not confer it.
pub(crate) fn writer() -> Result<AuthContext, Box<dyn std::error::Error>> {
    actor(&["workflow:content:write"])
}

pub(crate) fn submitter() -> Result<AuthContext, Box<dyn std::error::Error>> {
    actor(&["workflow:*"])
}

pub(crate) fn edit(
    expected_revision_digest: &str,
    client_mutation_id: &str,
    version: &str,
) -> InferenceProfileRevisionRequest {
    InferenceProfileRevisionRequest {
        schema_version: INFERENCE_PROFILE_REVISION_SCHEMA_VERSION.to_owned(),
        expected_revision_digest: expected_revision_digest.to_owned(),
        client_mutation_id: client_mutation_id.to_owned(),
        version: version.to_owned(),
        prompt_revision: "synthetic.prompt.v2".to_owned(),
        settings_revision: "synthetic.settings.v2".to_owned(),
        supported_settings: vec![
            "max_output_tokens".to_owned(),
            "max_provider_calls".to_owned(),
        ],
        effective_budgets: InferenceProfileBudgets {
            // Wide enough that the edited revision still admits the fixture
            // definition's limits, and still an edit: the budgets are part of
            // the revision's sealed identity.
            max_input_bytes: 128 * 1024,
            max_output_tokens: 8192,
            max_provider_calls: 64,
        },
    }
}

/// The pinned reference form a definition authors: `profile_id:version:digest`.
pub(crate) fn reference(descriptor: &InferenceProfileDescriptor) -> String {
    format!(
        "{}:{}:{}",
        descriptor.profile_id, descriptor.version, descriptor.digest
    )
}

pub(crate) fn parsed(definition: &Value) -> Result<WorkflowDefinition, Box<dyn std::error::Error>> {
    Ok(sts2_harness::management::decode_strict(
        &serde_json::to_vec(definition)?,
    )?)
}

/// The provenance reference persisted on an admitted run, read back from the
/// durable run record rather than from the submission response.
pub(crate) fn persisted_provenance(
    fixture: &Fixture,
    workflow_run_id: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let snapshot = fixture
        .store
        .get_run(workflow_run_id)?
        .ok_or("run snapshot missing after submission")?;
    snapshot
        .admission
        .and_then(|binding| binding.target.inference_profile)
        .ok_or_else(|| "resolved provenance was not persisted".into())
}

/// The new revision an admitted edit asks for, derived exactly as the service
/// derives it, for tests that drive the journal directly.
pub(crate) fn candidate(
    served: &InferenceProfileDescriptor,
    version: &str,
) -> Result<InferenceProfileDescriptor, Box<dyn std::error::Error>> {
    let request = edit(&served.digest, "mutation-durable", version);
    Ok(derive_inference_profile_revision(served, &request)
        .map_err(|error| std::io::Error::other(error.message))?)
}

/// The response an admitted edit produces, taken from the service itself rather
/// than hand-assembled, so the schema is checked against what the route serves.
pub(crate) fn adopted_response()
-> Result<InferenceProfileRevisionResponse, Box<dyn std::error::Error>> {
    let catalog = editable_catalog();
    let served = catalog.descriptors[0].clone();
    let fixture = super::inference_profile_catalog_doubles::fixture_with_revision_journal(
        CatalogCapabilityDouble::serving(vec![catalog]),
        &two_node_definition(DECIDE, PLANNER),
        "request-schema",
        Arc::new(MemoryInferenceProfileRevisionJournal::default()),
    )?;
    Ok(fixture.service.adopt_inference_profile_revision(
        &writer()?,
        DECIDE,
        edit(&served.digest, "mutation-schema", "1.1.0"),
    )?)
}
