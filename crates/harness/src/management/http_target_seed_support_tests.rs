// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

use super::super::HttpRequest;
use super::dispatch;
use crate::management::{
    AuthContext, CapabilityPort, ExecutionMode, ManagementError, ManagementService,
    MemoryWorkflowStore, StaticAuthenticator, TARGET_CATALOG_SCHEMA_VERSION, TargetAvailability,
    TargetCatalogResponse, TargetDescriptor, TargetSeedSupportCatalogV2,
};

const BEARER: &str = "target-seed-support-route-test";
const CATALOG_REVISION: &str = "runtime-v3:route-seed-support";

struct CatalogPort {
    catalog: TargetCatalogResponse,
    calls: Arc<AtomicUsize>,
}

impl CatalogPort {
    fn new() -> (Self, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            Self {
                catalog: TargetCatalogResponse {
                    schema_version: TARGET_CATALOG_SCHEMA_VERSION.to_owned(),
                    catalog_revision: CATALOG_REVISION.to_owned(),
                    targets: vec![TargetDescriptor {
                        instance_id: "route-target".to_owned(),
                        execution_profiles: vec!["live.workflow.v1".to_owned()],
                        execution_mode: ExecutionMode::Live,
                        compatibility_revision: "compat.v1".to_owned(),
                        capability_revision: "capabilities.v1".to_owned(),
                        availability: TargetAvailability::Available,
                        supported_operations: vec!["workflow:live".to_owned()],
                        capabilities: vec!["workflow:context:read".to_owned()],
                        game_profiles: vec!["sts2-live-v1".to_owned()],
                        save_profiles: Vec::new(),
                        inference_profiles: Vec::new(),
                    }],
                },
                calls: Arc::clone(&calls),
            },
            calls,
        )
    }
}

impl CapabilityPort for CatalogPort {
    fn capabilities(&self) -> Result<Value, ManagementError> {
        Ok(json!({"capabilities": []}))
    }

    fn target_catalog(
        &self,
        _actor: &AuthContext,
    ) -> Result<TargetCatalogResponse, ManagementError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.catalog.clone())
    }
}

fn service_and_calls() -> (ManagementService, Arc<AtomicUsize>) {
    let (catalog, calls) = CatalogPort::new();
    (
        ManagementService::new(Arc::new(MemoryWorkflowStore::default()))
            .with_capability_port(Arc::new(catalog)),
        calls,
    )
}

fn authenticator(scopes: &[&str]) -> StaticAuthenticator {
    let actor = AuthContext::new(
        "route-seed-support-reader",
        scopes.iter().map(|scope| (*scope).to_owned()),
    )
    .expect("valid route actor");
    StaticAuthenticator::new()
        .with_credential(BEARER, actor)
        .expect("route credential")
}

fn request(path: &str, query: BTreeMap<String, String>) -> HttpRequest {
    HttpRequest {
        method: "GET".to_owned(),
        path: path.to_owned(),
        query,
        headers: BTreeMap::from([("authorization".to_owned(), format!("Bearer {BEARER}"))]),
        body: Vec::new(),
    }
}

#[test]
fn authenticated_query_free_v2_route_returns_closed_support_and_keeps_v1() {
    let (service, calls) = service_and_calls();
    let read_auth = authenticator(&["workflow:read"]);
    let response = dispatch(
        request("/v2/workflow-targets", BTreeMap::new()),
        &service,
        &read_auth,
    )
    .expect("query-free support route");
    assert_eq!(response.status, 200);
    let support: TargetSeedSupportCatalogV2 =
        serde_json::from_slice(&response.body).expect("closed support response");
    assert_eq!(support.catalog_revision, CATALOG_REVISION);
    assert_eq!(support.targets.len(), 1);
    assert!(
        support.targets[0]
            .durable_candidate_binding_modes
            .is_empty()
    );
    assert!(support.targets[0].supported_launch_setups.is_empty());

    let query = BTreeMap::from([("filter".to_owned(), "live".to_owned())]);
    let error = dispatch(request("/v2/workflow-targets", query), &service, &read_auth)
        .expect_err("non-empty query must not alias the closed route");
    assert_eq!(error.code, "route_not_found");

    let v1 = dispatch(
        request("/v1/workflow-targets", BTreeMap::new()),
        &service,
        &read_auth,
    )
    .expect("existing v1 route");
    let body: Value = serde_json::from_slice(&v1.body).expect("V1 catalog body");
    assert_eq!(body["schema_version"], TARGET_CATALOG_SCHEMA_VERSION);
    assert!(body.get("support_revision").is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn v2_support_route_requires_authenticated_read_scope() {
    let (service, calls) = service_and_calls();
    let no_scope = authenticator(&[]);
    let error = dispatch(
        request("/v2/workflow-targets", BTreeMap::new()),
        &service,
        &no_scope,
    )
    .expect_err("support catalog requires workflow:read");
    assert_eq!(error.code, "missing_scope");
    assert_eq!(error.status(), 403);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
