// SPDX-License-Identifier: MIT

use std::net::SocketAddr;
use std::sync::Arc;

use sts2_harness::management::{
    AuthContext, ContextBindingCatalog, ContextBindingRequest, ContextOwnerPort, ManagementClient,
    ManagementServer, ServerConfig, SqliteWorkflowStore, StaticAuthenticator,
    SyntheticContextOwnerPort, decode_strict, synthetic_sqlite_store,
};

#[test]
fn synthetic_context_owner_advertises_one_metadata_binding_and_binds_it()
-> Result<(), Box<dyn std::error::Error>> {
    let owner = SyntheticContextOwnerPort;
    let actor = AuthContext::new("operator", ["workflow:*".to_owned()])?;
    let catalog = owner.catalog(&actor)?;
    catalog.validate()?;
    assert_eq!(catalog.descriptors.len(), 1);
    let descriptor = &catalog.descriptors[0];
    assert_eq!(descriptor.context_ref, "context.synthetic.v1");
    assert!(descriptor.grants.metadata_read);
    assert!(!descriptor.grants.content_read);
    assert!(descriptor.supports("context.synthetic.v1", "decide"));

    let request = ContextBindingRequest {
        workflow_run_id: "run.synthetic.1".to_owned(),
        definition_digest: "a".repeat(64),
        instance_id: "sts2-synthetic-1".to_owned(),
        graph_id: "main".to_owned(),
        node_id: "decide".to_owned(),
        node_execution_id: "run.synthetic.1.node.1".to_owned(),
        node_kind: "decide".to_owned(),
        context_ref: descriptor.context_ref.clone(),
        binding_id: descriptor.binding_id.clone(),
        binding_version: descriptor.version,
        binding_digest: descriptor.digest.clone(),
    };
    let binding = owner.bind(&actor, &request)?;
    binding.validate(None)?;
    assert_eq!(binding.workflow_run_id, "run.synthetic.1");

    let rejected = owner
        .bind(
            &actor,
            &ContextBindingRequest {
                context_ref: "context.other.v1".to_owned(),
                ..request.clone()
            },
        )
        .err();
    assert!(rejected.is_some());

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
fn context_binding_catalog_route_is_authenticated() -> Result<(), Box<dyn std::error::Error>> {
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
    assert_eq!(catalog.descriptors.len(), 1);
    assert_eq!(catalog.descriptors[0].context_ref, "context.synthetic.v1");

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
