// SPDX-License-Identifier: MIT

//! Composed authenticated context-owner effective limits over management HTTP.
//! Synthetic owner and in-memory store only; no provider or game is launched.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::sync::Arc;

use sts2_harness::management::{
    AuthContext, CONTEXT_OWNER_EFFECTIVE_LIMITS_VIEW_SCHEMA, ContextEffectiveLimits,
    MemoryWorkflowStore, StaticAuthenticator, UnavailableContextOwnerPort,
    compose_context_owner_binding, synthetic_store,
};

#[path = "support/context_owner_effective_limits.rs"]
mod fixture;

use fixture::{
    HARNESS_MAX_CONTROL_EVENTS, HARNESS_MAX_NOTES, Scenario, actor, binding_for, catalog_for,
    described_service, error_code, field, get, get_with, path_for, request_with, run_id,
    selected_limits, service,
};

#[test]
fn composed_limits_report_the_selected_values_for_the_current_binding() {
    let service = described_service(Scenario::Matching);
    let run = run_id(&service);
    let (status, value) = get(&service, &path_for(&run));
    assert_eq!(status, 200, "body: {value}");
    assert_eq!(
        field(&value, "schema_version").as_str(),
        Some(CONTEXT_OWNER_EFFECTIVE_LIMITS_VIEW_SCHEMA)
    );
    assert_eq!(
        field(&value, "owner_id").as_str(),
        Some("test.context-owner")
    );
    assert_eq!(field(&value, "owner_version").as_str(), Some("1.0.0"));
    assert_eq!(field(&value, "binding_id").as_str(), Some("test.binding.1"));
    assert_eq!(
        field(&value, "context_ref").as_str(),
        Some("context.test.v1")
    );
    assert_eq!(field(&value, "node_kind").as_str(), Some("decide"));
    assert_eq!(
        field(&value, "adapter_revision").as_str(),
        Some("test.adapter.v1")
    );
    assert_eq!(
        field(&value, "model_revision").as_str(),
        Some("test.model.v1")
    );
    let catalog_digest = field(&value, "catalog_digest")
        .as_str()
        .expect("catalog digest");
    assert_eq!(
        catalog_digest,
        catalog_for(Scenario::Matching).catalog_digest,
        "the projection must publish the digest of the catalog it composed"
    );
    let limits = field(&value, "effective_limits");
    assert_eq!(field(limits, "max_items").as_u64(), Some(2));
    assert_eq!(field(limits, "max_notes").as_u64(), Some(1));
    assert_eq!(field(limits, "max_context_bytes").as_u64(), Some(1024));
    assert_eq!(field(limits, "max_objective_bytes").as_u64(), Some(32));
    assert_eq!(field(limits, "max_control_events").as_u64(), Some(8));
    assert_ne!(
        field(limits, "max_notes").as_u64(),
        Some(HARNESS_MAX_NOTES),
        "selected limits must not be the harness maxima"
    );
    assert_ne!(
        field(limits, "max_control_events").as_u64(),
        Some(HARNESS_MAX_CONTROL_EVENTS)
    );
}

#[test]
fn a_binding_that_is_not_the_published_identity_is_refused() {
    let service = described_service(Scenario::DescriptorMismatch);
    let run = run_id(&service);
    let (status, value) = get(&service, &path_for(&run));
    assert_eq!(status, 409, "body: {value}");
    assert_eq!(
        error_code(&value),
        Some("context_owner_binding_descriptor_mismatch")
    );
}

#[test]
fn a_foreign_owner_binding_is_refused() {
    let service = described_service(Scenario::ForeignOwner);
    let run = run_id(&service);
    let (status, value) = get(&service, &path_for(&run));
    assert_eq!(status, 409, "body: {value}");
    assert_eq!(error_code(&value), Some("context_owner_binding_foreign"));
}

#[test]
fn a_binding_without_a_published_descriptor_is_refused() {
    let service = described_service(Scenario::MissingDescriptor);
    let run = run_id(&service);
    let (status, value) = get(&service, &path_for(&run));
    assert_eq!(status, 409, "body: {value}");
    assert_eq!(error_code(&value), Some("context_binding_unsupported"));
}

#[test]
fn a_disabled_descriptor_cannot_publish_limits() {
    let service = described_service(Scenario::DisabledDescriptor);
    let run = run_id(&service);
    let (status, value) = get(&service, &path_for(&run));
    assert_eq!(status, 409, "body: {value}");
    assert_eq!(error_code(&value), Some("context_binding_unsupported"));
}

#[test]
fn a_binding_that_is_not_available_cannot_publish_limits() {
    let service = described_service(Scenario::NonAvailableBinding);
    let run = run_id(&service);
    let (status, value) = get(&service, &path_for(&run));
    assert_eq!(status, 409, "body: {value}");
    assert_eq!(error_code(&value), Some("context_binding_unavailable"));
}

#[test]
fn an_escalating_binding_cannot_publish_limits() {
    let service = described_service(Scenario::EscalatingGrants);
    let run = run_id(&service);
    let (status, value) = get(&service, &path_for(&run));
    assert_eq!(status, 409, "body: {value}");
    assert_eq!(
        error_code(&value),
        Some("context_owner_binding_grant_escalation")
    );
}

#[test]
fn a_binding_for_another_run_is_refused() {
    let service = described_service(Scenario::ForeignRun);
    let run = run_id(&service);
    let (status, value) = get(&service, &path_for(&run));
    assert_eq!(status, 409, "body: {value}");
    assert_eq!(error_code(&value), Some("context_binding_mismatch"));
}

#[test]
fn an_oversized_descriptor_cannot_authorize_more_than_the_harness_ceiling() {
    let service = described_service(Scenario::OversizedDescriptor);
    let run = run_id(&service);
    let (status, value) = get(&service, &path_for(&run));
    assert_eq!(status, 400, "body: {value}");
    assert_eq!(error_code(&value), Some("context_effective_limits_invalid"));
}

#[test]
fn a_tampered_descriptor_cannot_authorize_more() {
    let service = described_service(Scenario::TamperedDescriptor);
    let run = run_id(&service);
    let (status, value) = get(&service, &path_for(&run));
    assert_eq!(status, 409, "body: {value}");
    assert_eq!(error_code(&value), Some("context_binding_digest_mismatch"));
}

#[test]
fn a_stale_catalog_digest_cannot_authorize_limits() {
    let service = described_service(Scenario::StaleCatalogDigest);
    let run = run_id(&service);
    let (status, value) = get(&service, &path_for(&run));
    assert_eq!(status, 409, "body: {value}");
    assert_eq!(error_code(&value), Some("context_catalog_digest_mismatch"));
}

#[test]
fn an_unattached_owner_stays_explicitly_unavailable() {
    let service = service(Arc::new(UnavailableContextOwnerPort));
    let run = run_id(&service);
    let (status, value) = get(&service, &path_for(&run));
    assert_eq!(status, 503, "body: {value}");
    assert_eq!(error_code(&value), Some("context_owner_unavailable"));
}

#[test]
fn an_owner_without_a_current_association_does_not_invent_limits() {
    let service = Arc::new(synthetic_store(Arc::new(MemoryWorkflowStore::new())));
    let run = run_id(&service);
    let (status, value) = get(&service, &path_for(&run));
    assert_eq!(status, 503, "body: {value}");
    assert_eq!(
        error_code(&value),
        Some("context_owner_association_unavailable")
    );
}

#[test]
fn missing_scope_and_run_scope_are_denied() {
    let service = described_service(Scenario::Matching);
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
}

#[test]
fn unknown_run_unknown_query_and_unknown_method_fail_closed() {
    let service = described_service(Scenario::Matching);
    let (status, value) = get(&service, &path_for("run.not.admitted"));
    assert_eq!(status, 400, "body: {value}");
    assert_eq!(error_code(&value), Some("run_not_found"));

    let run = run_id(&service);
    let (status, value) = get(&service, &format!("{}?limit=1", path_for(&run)));
    assert_eq!(status, 400, "body: {value}");
    assert_eq!(error_code(&value), Some("route_not_found"));

    let (status, value) = request_with(
        &service,
        "owner-token",
        StaticAuthenticator::single("owner-token", actor()).expect("authenticator"),
        "POST",
        &path_for(&run),
        Some(b"{}"),
    );
    assert_eq!(status, 400, "body: {value}");
    assert_eq!(error_code(&value), Some("route_not_found"));
}

#[test]
fn the_composition_seam_returns_the_published_descriptor() {
    let catalog = catalog_for(Scenario::Matching);
    let published = catalog.descriptors[0].clone();
    let binding = binding_for(Scenario::Matching, "run.compose.1", &published);
    let composed = compose_context_owner_binding(&catalog, &binding).expect("composed");
    assert_eq!(composed.binding_id, published.binding_id);
    assert_eq!(composed.version, published.version);
    assert_eq!(composed.digest, published.digest);
    assert_eq!(composed.effective_limits, selected_limits());
    assert_ne!(
        composed.effective_limits,
        ContextEffectiveLimits::default(),
        "the composed descriptor must not be the harness default"
    );
}

#[test]
fn the_composition_seam_refuses_a_binding_for_an_unpublished_kind() {
    let catalog = catalog_for(Scenario::Matching);
    let published = catalog.descriptors[0].clone();
    let mut binding = binding_for(Scenario::Matching, "run.compose.2", &published);
    binding.node_kind = "analyze".to_owned();
    let error = compose_context_owner_binding(&catalog, &binding).expect_err("refused");
    assert_eq!(error.code, "context_binding_unsupported");
}

/// Keeps the review-visible projection contract in sync with its own type: the
/// view serializes exactly the fields declared above and nothing else.
#[test]
fn the_projection_serializes_only_its_declared_fields() {
    let service = described_service(Scenario::Matching);
    let run = run_id(&service);
    let (_, value) = get(&service, &path_for(&run));
    let object = value.as_object().expect("object");
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "adapter_revision",
            "binding_digest",
            "binding_id",
            "binding_version",
            "catalog_digest",
            "context_ref",
            "effective_limits",
            "model_revision",
            "node_kind",
            "owner_id",
            "owner_version",
            "schema_version",
        ]
    );
}
