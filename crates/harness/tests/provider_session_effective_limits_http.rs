// SPDX-License-Identifier: MIT

//! Served provider-session effective-limits record over management HTTP.
//! Synthetic context owner, in-memory store and the library capability fixture
//! only; no provider or game is launched.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::Value;
use sts2_harness::effective_limits::{EFFECTIVE_LIMIT_RECORD_SCHEMA, EffectiveLimitRecord};
use sts2_harness::management::{
    AuthContext, ContextOwnerPort, ManagementClient, ManagementServer, ManagementService,
    MemoryWorkflowStore, ServerConfig, StaticAuthenticator, synthetic_store,
};
use sts2_harness::provider_session::NativeCapabilities;

#[allow(dead_code)]
#[path = "support/context_owner_effective_limits.rs"]
mod fixture;

use fixture::{OwnerDouble, Scenario, actor, error_code, get, get_with, request_with, run_id};

const OWNER_TOKEN: &str = "owner-token";
const SERVED_MAX_OPERATIONS: usize = 7;

/// The descriptor the served composition admits provider sessions against. It
/// binds the adapter and model revision the synthetic owner's boundary names,
/// and one executable ceiling is deliberately below the library fixture value
/// so a route that answered with a hard-coded fixture record would be visibly
/// wrong.
fn served_capabilities() -> NativeCapabilities {
    let mut capabilities = NativeCapabilities::fixture();
    capabilities.profile_id = "test.adapter.v1".to_owned();
    capabilities.profile_sha256 = sts2_harness::sha256_hex("test.adapter.v1");
    capabilities.native_version = "test.model.v1".to_owned();
    capabilities.binding.adapter_revision = "test.adapter.v1".to_owned();
    capabilities.binding.adapter_revision_sha256 = capabilities.profile_sha256.clone();
    capabilities.binding.model_revision = "test.model.v1".to_owned();
    capabilities.effective_limits.max_operations = SERVED_MAX_OPERATIONS;
    capabilities.binding.descriptor_sha256 = capabilities.descriptor_digest();
    capabilities
        .validate()
        .expect("served capabilities are a valid descriptor");
    capabilities
}

fn served_service(
    owner: Arc<dyn ContextOwnerPort>,
    capabilities: NativeCapabilities,
) -> Arc<ManagementService> {
    Arc::new(
        synthetic_store(Arc::new(MemoryWorkflowStore::new()))
            .with_context_owner_port(owner)
            .with_provider_session_capabilities(capabilities)
            .expect("descriptor attaches"),
    )
}

fn matching_service() -> Arc<ManagementService> {
    served_service(
        Arc::new(OwnerDouble::new(Scenario::Matching)),
        served_capabilities(),
    )
}

fn path_for(run_id: &str) -> String {
    format!("/v1/workflow-runs/{run_id}/provider-session-effective-limits")
}

fn memory_path_for(run_id: &str) -> String {
    format!("/v1/workflow-runs/{run_id}/context-memory-effective-limits")
}

/// Raw response bytes, so the served encoding can be compared to the producer's.
fn get_bytes(service: &Arc<ManagementService>, path: &str) -> (u16, Vec<u8>) {
    let authenticator = StaticAuthenticator::single(OWNER_TOKEN, actor()).expect("authenticator");
    let config = ServerConfig::new(
        "127.0.0.1:0".parse().expect("loopback"),
        Arc::new(authenticator),
    )
    .expect("server config");
    let server = ManagementServer::start(config, Arc::clone(service)).expect("server starts");
    let client = ManagementClient::new(server.address(), OWNER_TOKEN).expect("client");
    let response = client.request_json("GET", path, None).expect("response");
    server.shutdown().expect("server shuts down");
    (response.status, response.body)
}

fn keys(value: &Value) -> BTreeSet<&str> {
    value
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect()
}

#[test]
fn route_returns_producer_record_matching_library() {
    let service = matching_service();
    let run = run_id(&service);
    let (status, body) = get_bytes(&service, &path_for(&run));
    assert_eq!(status, 200, "body: {}", String::from_utf8_lossy(&body));

    let produced = served_capabilities().effective_limit_record();
    produced.validate().expect("producer record validates");
    // The management surface encodes every response through `serde_json::Value`;
    // the served bytes must be exactly that encoding of the producer's record.
    let expected = serde_json::to_vec(&serde_json::to_value(&produced).expect("value"))
        .expect("encoded producer record");
    assert_eq!(body, expected, "served bytes are not the producer's record");

    let served: EffectiveLimitRecord = serde_json::from_slice(&body).expect("typed record");
    assert_eq!(served, produced);
    let operations = served
        .rows
        .iter()
        .find(|row| row.field == "max_operations")
        .expect("max_operations row");
    assert_eq!(operations.executable_ceiling, SERVED_MAX_OPERATIONS as u64);
    assert_ne!(
        served,
        NativeCapabilities::fixture().effective_limit_record(),
        "the route must publish the served descriptor, not the library fixture"
    );
    assert_eq!(
        served.capability_descriptor_sha256,
        served_capabilities().binding.descriptor_sha256
    );
}

#[test]
fn record_is_metadata_only() {
    let service = matching_service();
    let run = run_id(&service);
    let (status, body) = get_bytes(&service, &path_for(&run));
    assert_eq!(status, 200);
    let value: Value = serde_json::from_slice(&body).expect("JSON");
    assert_eq!(
        keys(&value),
        [
            "capability_descriptor_sha256",
            "capability_schema",
            "enabled",
            "owner",
            "owner_revision",
            "rows",
            "schema",
            "surface",
        ]
        .into_iter()
        .collect::<BTreeSet<_>>()
    );
    assert_eq!(value["schema"], EFFECTIVE_LIMIT_RECORD_SCHEMA);
    assert_eq!(value["surface"], "provider-session");
    let rows = value["rows"].as_array().expect("rows");
    assert!(!rows.is_empty());
    for row in rows {
        assert_eq!(
            keys(row),
            [
                "capabilities_schema_ceiling",
                "class",
                "executable_ceiling",
                "field",
                "policy_schema_ceiling",
                "validator",
            ]
            .into_iter()
            .collect::<BTreeSet<_>>()
        );
    }
    // Nothing from the descriptor beyond its classification and digest, and no
    // credential or session content, reaches the wire.
    let text = String::from_utf8(body).expect("utf-8");
    let capabilities = served_capabilities();
    for private in [
        OWNER_TOKEN,
        "enabled_methods",
        "hardening",
        "transport",
        "policy_bytes",
        "content",
        capabilities.native_binary_sha256.as_str(),
        capabilities.profile_sha256.as_str(),
    ] {
        assert!(
            !text.contains(private),
            "served record must not carry {private}"
        );
    }
}

#[test]
fn route_requires_read_scope_and_current_association() {
    let service = matching_service();
    let run = run_id(&service);

    let unprivileged =
        AuthContext::new("integration.tester", ["events:read".to_owned()]).expect("actor");
    let (status, value) = get_with(
        &service,
        "unprivileged-token",
        StaticAuthenticator::single("unprivileged-token", unprivileged).expect("authenticator"),
        "GET",
        &path_for(&run),
    );
    assert_eq!(status, 403, "body: {value}");
    assert_eq!(error_code(&value), Some("missing_scope"));

    let scoped = AuthContext::with_run_prefix(
        "integration.tester",
        ["workflow:read".to_owned()],
        Some("some.other.run".to_owned()),
    )
    .expect("actor");
    let (status, value) = get_with(
        &service,
        "scoped-token",
        StaticAuthenticator::single("scoped-token", scoped).expect("authenticator"),
        "GET",
        &path_for(&run),
    );
    assert_eq!(status, 403, "body: {value}");
    assert_eq!(error_code(&value), Some("run_scope_denied"));

    let (status, value) = get(&service, &path_for("run.not.admitted"));
    assert_eq!(status, 400, "body: {value}");
    assert_eq!(error_code(&value), Some("run_not_found"));

    // No current association: the unattached owner stays explicitly unavailable.
    let unattached = Arc::new(
        synthetic_store(Arc::new(MemoryWorkflowStore::new()))
            .with_provider_session_capabilities(served_capabilities())
            .expect("descriptor attaches"),
    );
    let run_without_owner = run_id(&unattached);
    let (status, value) = get(&unattached, &path_for(&run_without_owner));
    assert_eq!(status, 503, "body: {value}");
    assert_eq!(
        error_code(&value),
        Some("context_owner_association_unavailable")
    );

    // An association for another run is refused before any record is built.
    let foreign = served_service(
        Arc::new(OwnerDouble::new(Scenario::ForeignRun)),
        served_capabilities(),
    );
    let foreign_run = run_id(&foreign);
    let (status, value) = get(&foreign, &path_for(&foreign_run));
    assert_eq!(status, 409, "body: {value}");
    assert_eq!(error_code(&value), Some("context_binding_mismatch"));

    // An association bound to another adapter/model revision than the served
    // descriptor is refused: the record would describe a different profile.
    let mismatched = served_service(
        Arc::new(OwnerDouble::new(Scenario::Matching)),
        NativeCapabilities::fixture(),
    );
    let mismatched_run = run_id(&mismatched);
    let (status, value) = get(&mismatched, &path_for(&mismatched_run));
    assert_eq!(status, 409, "body: {value}");
    assert_eq!(
        error_code(&value),
        Some("provider_session_capabilities_mismatch")
    );

    // A composition without a served descriptor never answers with a fixture.
    let no_descriptor = Arc::new(
        synthetic_store(Arc::new(MemoryWorkflowStore::new()))
            .with_context_owner_port(Arc::new(OwnerDouble::new(Scenario::Matching))),
    );
    let bare_run = run_id(&no_descriptor);
    let (status, value) = get(&no_descriptor, &path_for(&bare_run));
    assert_eq!(status, 503, "body: {value}");
    assert_eq!(
        error_code(&value),
        Some("provider_session_capabilities_unavailable")
    );

    // Unknown query and method fail closed like the sibling routes.
    let (status, value) = get(&service, &format!("{}?limit=1", path_for(&run)));
    assert_eq!(status, 400, "body: {value}");
    assert_eq!(error_code(&value), Some("route_not_found"));
    let (status, value) = request_with(
        &service,
        OWNER_TOKEN,
        StaticAuthenticator::single(OWNER_TOKEN, actor()).expect("authenticator"),
        "POST",
        &path_for(&run),
        Some(b"{}"),
    );
    assert_eq!(status, 400, "body: {value}");
    assert_eq!(error_code(&value), Some("route_not_found"));
}

#[test]
fn memory_record_is_explicitly_unsupported_or_served() {
    let service = matching_service();
    let run = run_id(&service);

    // The served composition holds no memory corpus: the record is refused with
    // a typed code after the same scope and run checks, never faked.
    let (status, value) = get(&service, &memory_path_for(&run));
    assert_eq!(status, 503, "body: {value}");
    assert_eq!(
        error_code(&value),
        Some("context_memory_record_unavailable")
    );

    let (status, value) = get(&service, &memory_path_for("run.not.admitted"));
    assert_eq!(status, 400, "body: {value}");
    assert_eq!(error_code(&value), Some("run_not_found"));

    let unprivileged =
        AuthContext::new("integration.tester", ["events:read".to_owned()]).expect("actor");
    let (status, value) = get_with(
        &service,
        "unprivileged-token",
        StaticAuthenticator::single("unprivileged-token", unprivileged).expect("authenticator"),
        "GET",
        &memory_path_for(&run),
    );
    assert_eq!(status, 403, "body: {value}");
    assert_eq!(error_code(&value), Some("missing_scope"));
}
