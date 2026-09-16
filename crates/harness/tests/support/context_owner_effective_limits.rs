// SPDX-License-Identifier: MIT

//! Fixtures for the composed authenticated context-owner effective-limits
//! surface. Synthetic owner and in-memory store only; no provider or game is
//! launched. Shared by the HTTP route suite and the composition seam suite.

use std::sync::Arc;

use serde_json::Value;
use sts2_harness::context_control::ContextBoundary;
use sts2_harness::management::{
    AuthContext, CONTEXT_OWNER_BINDING_SCHEMA_VERSION, CONTEXT_OWNER_CATALOG_SCHEMA_VERSION,
    ContextBindingCatalog, ContextBindingContinuity, ContextBindingDescriptor,
    ContextBindingGrants, ContextBindingRequest, ContextBindingState, ContextEffectiveLimits,
    ContextOwnerBinding, ContextOwnerPort, MANAGEMENT_SCHEMA_VERSION, ManagementClient,
    ManagementError, ManagementServer, ManagementService, MemoryWorkflowStore, RunRequest,
    ServerConfig, StaticAuthenticator, synthetic_store,
};

const VALID_WORKFLOW: &[u8] =
    include_bytes!("../../../../conformance/workflow-v1/valid-strict.json");
pub const HARNESS_MAX_NOTES: u64 = 16;
pub const HARNESS_MAX_CONTROL_EVENTS: u64 = 4096;

/// Deliberately restricted, schema-valid advertised limits. Every value is
/// below the harness maximum so a composed projection that reported the maxima
/// instead of the selected values would be visibly wrong.
pub fn selected_limits() -> ContextEffectiveLimits {
    ContextEffectiveLimits {
        max_items: 2,
        max_notes: 1,
        max_context_bytes: 1024,
        max_objective_bytes: 32,
        max_control_events: 8,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Scenario {
    /// The owner's current binding is exactly the published descriptor.
    Matching,
    /// The binding claims an owner the catalog does not publish.
    ForeignOwner,
    /// The binding identity is not the published descriptor identity.
    DescriptorMismatch,
    /// No descriptor is published for the binding's context reference.
    MissingDescriptor,
    /// The published descriptor is disabled while the binding claims availability.
    DisabledDescriptor,
    /// The binding is not currently available.
    NonAvailableBinding,
    /// The binding escalates a grant beyond its admitting descriptor.
    EscalatingGrants,
    /// The owner answers for a different workflow run.
    ForeignRun,
    /// The catalog advertises a value above the harness ceiling.
    OversizedDescriptor,
    /// The descriptor's declared digest no longer matches its limits.
    TamperedDescriptor,
    /// The catalog's declared digest does not match its descriptors.
    StaleCatalogDigest,
}

fn descriptor(
    limits: ContextEffectiveLimits,
    state: ContextBindingState,
) -> ContextBindingDescriptor {
    ContextBindingDescriptor {
        schema_version: CONTEXT_OWNER_BINDING_SCHEMA_VERSION.to_owned(),
        binding_id: "test.binding.1".to_owned(),
        version: 1,
        digest: String::new(),
        context_ref: "context.test.v1".to_owned(),
        node_kinds: vec!["decide".to_owned()],
        sources: Vec::new(),
        operations: Vec::new(),
        effective_limits: limits,
        continuity: ContextBindingContinuity {
            survives_controller_restart: true,
            receipt_recovery: true,
            provider_session_continuity: false,
        },
        grants: grants(),
        state,
    }
    .seal()
    .expect("seal descriptor")
}

fn grants() -> ContextBindingGrants {
    ContextBindingGrants {
        metadata_read: true,
        content_read: false,
        edit: false,
        control: false,
    }
}

/// Builds the scenario catalog without validating it: the service is the unit
/// under test and must reject an oversized, tampered or stale catalog itself.
pub fn catalog_for(scenario: Scenario) -> ContextBindingCatalog {
    let published = match scenario {
        Scenario::OversizedDescriptor => descriptor(
            ContextEffectiveLimits {
                max_notes: HARNESS_MAX_NOTES + 1,
                ..selected_limits()
            },
            ContextBindingState::Available,
        ),
        Scenario::TamperedDescriptor => {
            let mut value = descriptor(selected_limits(), ContextBindingState::Available);
            value.effective_limits.max_notes = 2;
            value
        }
        Scenario::DisabledDescriptor => {
            descriptor(selected_limits(), ContextBindingState::Disabled)
        }
        _ => descriptor(selected_limits(), ContextBindingState::Available),
    };
    let catalog = ContextBindingCatalog {
        schema_version: CONTEXT_OWNER_CATALOG_SCHEMA_VERSION.to_owned(),
        owner_id: "test.context-owner".to_owned(),
        owner_version: "1.0.0".to_owned(),
        catalog_digest: String::new(),
        descriptors: vec![published],
    }
    .seal()
    .expect("seal catalog");
    match scenario {
        Scenario::StaleCatalogDigest => {
            let mut stale = catalog;
            stale.catalog_digest = "a".repeat(64);
            stale
        }
        _ => catalog,
    }
}

fn boundary(run_id: &str) -> ContextBoundary {
    ContextBoundary {
        run_id: run_id.to_owned(),
        episode_id: "test.episode.1".to_owned(),
        agent_id: "test.agent.1".to_owned(),
        state_id: "test.state.1".to_owned(),
        generation: 1,
        observation_sha256: "b".repeat(64),
        catalog_sha256: "c".repeat(64),
        adapter_revision: "test.adapter.v1".to_owned(),
        model_revision: "test.model.v1".to_owned(),
        configuration_sha256: "d".repeat(64),
        output_schema_sha256: "e".repeat(64),
        controller_epoch: 1,
        gate_epoch: 1,
        control_version: 1,
    }
}

pub fn binding_for(
    scenario: Scenario,
    run_id: &str,
    published: &ContextBindingDescriptor,
) -> ContextOwnerBinding {
    let answer_for = match scenario {
        Scenario::ForeignRun => "run.somewhere.else",
        _ => run_id,
    };
    let mut binding = ContextOwnerBinding {
        schema_version: CONTEXT_OWNER_BINDING_SCHEMA_VERSION.to_owned(),
        owner_id: "test.context-owner".to_owned(),
        owner_version: "1.0.0".to_owned(),
        invocation_id: "test.invocation.1".to_owned(),
        binding_id: published.binding_id.clone(),
        binding_version: published.version,
        binding_digest: published.digest.clone(),
        context_ref: published.context_ref.clone(),
        instance_id: "sts2-test-1".to_owned(),
        node_kind: "decide".to_owned(),
        state: ContextBindingState::Available,
        workflow_run_id: answer_for.to_owned(),
        definition_digest: "f".repeat(64),
        graph_id: "main".to_owned(),
        node_id: "decide".to_owned(),
        node_execution_id: "test.node.1".to_owned(),
        boundary: boundary(answer_for),
        lease_epoch: 1,
        snapshot_id: "test.snapshot.1".to_owned(),
        approved_revision_id: "test.revision.1".to_owned(),
        plan_epoch: 1,
        grants: grants(),
        continuity: published.continuity.clone(),
    };
    match scenario {
        Scenario::ForeignOwner => binding.owner_id = "other.context-owner".to_owned(),
        Scenario::DescriptorMismatch => binding.binding_digest = "9".repeat(64),
        Scenario::MissingDescriptor => binding.context_ref = "context.unpublished.v1".to_owned(),
        Scenario::NonAvailableBinding => binding.state = ContextBindingState::Denied,
        Scenario::EscalatingGrants => binding.grants.control = true,
        _ => {}
    }
    binding
}

/// Authoritative-owner double. It answers `catalog` and the **current**
/// association, so the composed path is exercised without a real owner
/// transport; `bind` is deliberately unused here.
pub struct OwnerDouble {
    scenario: Scenario,
}

impl OwnerDouble {
    pub fn new(scenario: Scenario) -> Self {
        Self { scenario }
    }
}

impl ContextOwnerPort for OwnerDouble {
    fn catalog(&self, _actor: &AuthContext) -> Result<ContextBindingCatalog, ManagementError> {
        Ok(catalog_for(self.scenario))
    }

    fn bind(
        &self,
        _actor: &AuthContext,
        _request: &ContextBindingRequest,
    ) -> Result<ContextOwnerBinding, ManagementError> {
        Err(ManagementError::unavailable(
            "test_bind_unused",
            "bind is not used by the effective-limits path",
        ))
    }

    fn association(
        &self,
        _actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
    ) -> Result<ContextOwnerBinding, ManagementError> {
        let published = catalog_for(self.scenario).descriptors[0].clone();
        Ok(binding_for(
            self.scenario,
            &snapshot.workflow_run_id,
            &published,
        ))
    }

    fn is_available(&self) -> bool {
        true
    }
}

pub fn actor() -> AuthContext {
    AuthContext::new("integration.tester", ["workflow:*".to_owned()]).expect("actor")
}

fn request() -> RunRequest {
    RunRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        request_id: "effective-limits-request".to_owned(),
        definition: Some(serde_json::from_slice::<Value>(VALID_WORKFLOW).expect("fixture")),
        artifact_id: None,
        instance_id: "integration-instance".to_owned(),
        profile: "synthetic".to_owned(),
        admission: None,
    }
}

pub fn service(owner: Arc<dyn ContextOwnerPort>) -> Arc<ManagementService> {
    Arc::new(synthetic_store(Arc::new(MemoryWorkflowStore::new())).with_context_owner_port(owner))
}

pub fn described_service(scenario: Scenario) -> Arc<ManagementService> {
    service(Arc::new(OwnerDouble::new(scenario)))
}

pub fn run_id(service: &ManagementService) -> String {
    service
        .submit_run(&actor(), request())
        .expect("run is admitted")
        .workflow_run_id
}

pub fn path_for(run_id: &str) -> String {
    format!("/v1/workflow-runs/{run_id}/context-owner-effective-limits")
}

pub fn get_with(
    service: &Arc<ManagementService>,
    token: &str,
    authenticator: StaticAuthenticator,
    method: &str,
    path: &str,
) -> (u16, Value) {
    request_with(service, token, authenticator, method, path, None)
}

pub fn request_with(
    service: &Arc<ManagementService>,
    token: &str,
    authenticator: StaticAuthenticator,
    method: &str,
    path: &str,
    body: Option<&[u8]>,
) -> (u16, Value) {
    let config = ServerConfig::new(
        "127.0.0.1:0".parse().expect("loopback"),
        Arc::new(authenticator),
    )
    .expect("server config");
    let server = ManagementServer::start(config, Arc::clone(service)).expect("server starts");
    let client = ManagementClient::new(server.address(), token).expect("client");
    let response = client.request_json(method, path, body).expect("response");
    server.shutdown().expect("server shuts down");
    let value: Value = serde_json::from_slice(&response.body).expect("JSON response");
    (response.status, value)
}

pub fn get(service: &Arc<ManagementService>, path: &str) -> (u16, Value) {
    get_with(
        service,
        "owner-token",
        StaticAuthenticator::single("owner-token", actor()).expect("authenticator"),
        "GET",
        path,
    )
}

pub fn error_code(value: &Value) -> Option<&str> {
    value
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
}

pub fn field<'a>(value: &'a Value, name: &str) -> &'a Value {
    value.get(name).unwrap_or_else(|| panic!("missing {name}"))
}
