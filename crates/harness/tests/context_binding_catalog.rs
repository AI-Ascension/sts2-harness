// SPDX-License-Identifier: MIT

use std::net::SocketAddr;
use std::sync::Arc;

use sts2_harness::management::{
    AuthContext, ContextBindingCatalog, ContextBindingDescriptor, ContextBindingRequest,
    ContextOwnerBinding, ContextOwnerPort, ManagementClient, ManagementServer, ServerConfig,
    SqliteWorkflowStore, StaticAuthenticator, SyntheticContextOwnerPort, decode_strict,
    synthetic_sqlite_store,
};

fn request_for(descriptor: &ContextBindingDescriptor) -> ContextBindingRequest {
    ContextBindingRequest {
        workflow_run_id: "run.synthetic.1".to_owned(),
        definition_digest: "a".repeat(64),
        instance_id: "sts2-synthetic-1".to_owned(),
        graph_id: "main".to_owned(),
        node_id: "decide".to_owned(),
        node_execution_id: "run.synthetic.1.node.1".to_owned(),
        node_kind: descriptor.node_kinds[0].clone(),
        context_ref: descriptor.context_ref.clone(),
        binding_id: descriptor.binding_id.clone(),
        binding_version: descriptor.version,
        binding_digest: descriptor.digest.clone(),
    }
}

#[test]
fn synthetic_context_owner_advertises_metadata_bindings_and_binds_them()
-> Result<(), Box<dyn std::error::Error>> {
    let owner = SyntheticContextOwnerPort;
    let actor = AuthContext::new("operator", ["workflow:*".to_owned()])?;
    let catalog = owner.catalog(&actor)?;
    catalog.validate()?;
    assert_eq!(catalog.descriptors.len(), 10);
    let descriptor = catalog
        .descriptors
        .iter()
        .find(|descriptor| descriptor.context_ref == "context.synthetic.v1")
        .ok_or("synthetic context ref missing")?;
    assert!(descriptor.grants.metadata_read);
    assert!(!descriptor.grants.content_read);
    assert!(descriptor.supports("context.synthetic.v1", "decide"));
    assert!(
        catalog
            .descriptors
            .iter()
            .any(|descriptor| descriptor.context_ref == "sts2.combat.context.v1")
    );

    let request = request_for(descriptor);
    let binding = owner.bind(&actor, &request)?;
    binding.validate(None)?;
    assert_eq!(binding.workflow_run_id, "run.synthetic.1");

    let foreign = owner
        .bind(
            &actor,
            &ContextBindingRequest {
                context_ref: "context.other.v1".to_owned(),
                ..request.clone()
            },
        )
        .err();
    assert!(foreign.is_some());

    let version_mismatch = owner
        .bind(
            &actor,
            &ContextBindingRequest {
                binding_version: descriptor.version + 1,
                ..request
            },
        )
        .err();
    assert!(version_mismatch.is_some());
    Ok(())
}

#[test]
fn context_binding_routes_are_authenticated_and_owner_validated()
-> Result<(), Box<dyn std::error::Error>> {
    let directory =
        std::env::temp_dir().join(format!("sts2-context-binding-{}", std::process::id()));
    std::fs::create_dir_all(&directory)?;
    let store = SqliteWorkflowStore::open(directory.join("store.sqlite"))?;
    let service = Arc::new(synthetic_sqlite_store(Arc::new(store)));
    let operator = AuthContext::new("operator", ["workflow:*".to_owned()])?;
    let authenticator = StaticAuthenticator::new().with_credential("operator-token", operator)?;
    let config = ServerConfig::new(
        "127.0.0.1:0".parse::<SocketAddr>()?,
        Arc::new(authenticator),
    )?;
    let server = ManagementServer::start(config, service)?;
    let client = ManagementClient::new(server.address(), "operator-token")?;

    let response = client.request_json("GET", "/v1/context-bindings", None)?;
    assert_eq!(response.status, 200);
    let catalog: ContextBindingCatalog = decode_strict(&response.body)?;
    catalog.validate()?;
    assert_eq!(catalog.descriptors.len(), 10);
    let descriptor = catalog
        .descriptors
        .iter()
        .find(|descriptor| descriptor.context_ref == "sts2.combat.context.v1")
        .ok_or("combat context ref missing")?;

    let body = serde_json::to_vec(&request_for(descriptor))?;
    let bound = client.request_json("POST", "/v1/context-bindings/bind", Some(&body))?;
    assert_eq!(bound.status, 200);
    let binding: ContextOwnerBinding = decode_strict(&bound.body)?;
    binding.validate(None)?;
    assert_eq!(binding.context_ref, "sts2.combat.context.v1");

    let unauthorized = ManagementClient::new(server.address(), "unknown-token")?.request_json(
        "GET",
        "/v1/context-bindings",
        None,
    )?;
    assert_eq!(unauthorized.status, 401);

    server.shutdown()?;
    let _ = std::fs::remove_dir_all(&directory);
    Ok(())
}

struct MiscorrelatedOwner;

impl ContextOwnerPort for MiscorrelatedOwner {
    fn catalog(
        &self,
        actor: &AuthContext,
    ) -> Result<ContextBindingCatalog, sts2_harness::management::ManagementError> {
        SyntheticContextOwnerPort.catalog(actor)
    }

    fn bind(
        &self,
        actor: &AuthContext,
        request: &ContextBindingRequest,
    ) -> Result<ContextOwnerBinding, sts2_harness::management::ManagementError> {
        let mut binding = SyntheticContextOwnerPort.bind(actor, request)?;
        binding.node_id = "miscorrelated.node".to_owned();
        Ok(binding)
    }
}

fn valid_request(run_id: &str) -> Result<ContextBindingRequest, Box<dyn std::error::Error>> {
    let owner = SyntheticContextOwnerPort;
    let actor = AuthContext::new("operator", ["workflow:*".to_owned()])?;
    let catalog = owner.catalog(&actor)?;
    let descriptor = catalog
        .descriptors
        .iter()
        .find(|descriptor| descriptor.context_ref == "context.synthetic.v1")
        .ok_or("synthetic context ref missing")?;
    Ok(ContextBindingRequest {
        workflow_run_id: run_id.to_owned(),
        ..request_for(descriptor)
    })
}

#[test]
fn bind_context_correlates_owner_response() -> Result<(), Box<dyn std::error::Error>> {
    let service = sts2_harness::management::ManagementService::in_memory()
        .with_context_owner_port(Arc::new(MiscorrelatedOwner));
    let actor = AuthContext::new("operator", ["workflow:*".to_owned()])?;
    let error = service
        .bind_context(&actor, valid_request("run.synthetic.1")?)
        .err();
    assert_eq!(
        error.map(|error| error.code),
        Some("context_binding_mismatch".to_owned())
    );
    Ok(())
}

#[test]
fn bind_context_enforces_run_scope() -> Result<(), Box<dyn std::error::Error>> {
    let service = sts2_harness::management::ManagementService::in_memory();
    let actor = AuthContext::with_run_prefix(
        "reader",
        ["workflow:read".to_owned()],
        Some("run.allowed".to_owned()),
    )?;
    let error = service
        .bind_context(&actor, valid_request("run.denied")?)
        .err();
    assert_eq!(
        error.map(|error| error.code),
        Some("run_scope_denied".to_owned())
    );
    Ok(())
}
