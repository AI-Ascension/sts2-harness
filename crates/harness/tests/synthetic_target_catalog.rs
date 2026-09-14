// SPDX-License-Identifier: MIT

use std::net::SocketAddr;
use std::sync::Arc;

use sts2_harness::management::{
    AuthContext, ExecutionMode, ManagementClient, ManagementServer, RunTargetConfiguration,
    ServerConfig, SqliteWorkflowStore, StaticAuthenticator, TargetAdmissionRequest,
    TargetCatalogResponse, TargetPreflightResponse, decode_strict, synthetic_sqlite_store,
};

/// The deterministic synthetic owner must publish exactly one clearly-labelled
/// synthetic target so an authenticated Studio consumer can discover a target,
/// preflight an exact admission, and submit without any game, provider, or lease
/// authority. This is the companion contract for ascension-workflow-studio#107.
#[test]
fn synthetic_owner_serves_a_scoped_target_catalog_and_preflight()
-> Result<(), Box<dyn std::error::Error>> {
    let directory =
        std::env::temp_dir().join(format!("sts2-synthetic-target-{}", std::process::id()));
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

    let response = client.request_json("GET", "/v1/workflow-targets", None)?;
    assert_eq!(response.status, 200);
    let catalog: TargetCatalogResponse = decode_strict(&response.body)?;
    catalog.validate()?;
    assert_eq!(catalog.targets.len(), 1);
    let descriptor = &catalog.targets[0];
    assert_eq!(descriptor.execution_mode, ExecutionMode::Synthetic);
    assert_eq!(descriptor.execution_profiles, vec!["synthetic".to_owned()]);
    assert_eq!(
        descriptor.game_profiles,
        vec!["sts2-synthetic-v1".to_owned()]
    );

    let target = RunTargetConfiguration {
        instance_id: descriptor.instance_id.clone(),
        execution_profile: descriptor.execution_profiles[0].clone(),
        execution_mode: ExecutionMode::Synthetic,
        workflow_revision: "1.0.0".to_owned(),
        compatibility_revision: descriptor.compatibility_revision.clone(),
        capability_revision: descriptor.capability_revision.clone(),
        game_profile: descriptor.game_profiles[0].clone(),
        save_profile: None,
        inference_profile: None,
        context_capability: None,
        provider_capability: None,
    };
    let body = serde_json::to_vec(&TargetAdmissionRequest {
        schema_version: "ascension.workflow-admission/v1".to_owned(),
        request_id: "request-synthetic-catalog".to_owned(),
        workflow_definition_digest: "a".repeat(64),
        target,
    })?;
    let preflight_response =
        client.request_json("POST", "/v1/workflow-targets/preflight", Some(&body))?;
    assert_eq!(preflight_response.status, 200);
    let preflight: TargetPreflightResponse = decode_strict(&preflight_response.body)?;
    preflight.validate()?;
    assert_eq!(
        preflight.admission.target.instance_id,
        descriptor.instance_id
    );

    server.shutdown()?;
    let _ = std::fs::remove_dir_all(&directory);
    Ok(())
}
