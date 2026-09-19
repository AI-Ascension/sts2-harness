// SPDX-License-Identifier: MIT

//! Inference-profile catalog, typed binding and admission gates (#104).
//!
//! Everything here is synthetic and labelled as such: the catalogs are
//! in-memory owner doubles, the execution port only records that it was called,
//! and no provider, model, credential or native host is contacted. A pass is
//! source/component evidence for the admission fences, not provider-execution
//! evidence.

#![allow(clippy::expect_used)]

use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde_json::{Value, json};
use sts2_harness::management::{
    AuthContext, INFERENCE_PROFILE_PROVENANCE_PREFIX, InferenceProfileState,
    LiveInferenceProfileCatalogPort, ManagementClient, ManagementError, ManagementServer,
    ManagementService, RunSubmissionResponse, ServerConfig, StaticAuthenticator, decode_strict,
};
use sts2_harness::workflow::WorkflowDefinition;

#[path = "support/live_workflow_context_owner.rs"]
mod context_owner_double;
#[path = "support/inference_profile_catalog_doubles.rs"]
mod inference_profile_catalog_doubles;
#[path = "support/inference_profile_catalog_fixtures.rs"]
mod inference_profile_catalog_fixtures;

use inference_profile_catalog_doubles::*;
use inference_profile_catalog_fixtures::*;

const WORKFLOW_DEFINITION: &[u8] =
    include_bytes!("../../../conformance/workflow-v1/valid-strict.json");

fn actor() -> Result<AuthContext, Box<dyn std::error::Error>> {
    Ok(AuthContext::new("operator", ["workflow:*".to_owned()])?)
}

fn submit(fixture: &Fixture) -> Result<RunSubmissionResponse, ManagementError> {
    fixture
        .service
        .submit_run(&actor().expect("actor"), fixture.request.clone())
}

#[test]
fn two_node_bindings_resolve_distinct_exact_profiles_and_persist_credential_free_provenance()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture(
        CatalogCapabilityDouble::serving(vec![baseline_catalog()]),
        &two_node_definition("decision.synthetic.v1", "planner.synthetic.v1"),
        "request-two-node",
    )?;
    let response = submit(&fixture)?;

    let persisted = fixture
        .store
        .get_run(&response.workflow_run_id)?
        .ok_or("run snapshot missing after submission")?;
    let provenance = persisted
        .admission
        .as_ref()
        .and_then(|binding| binding.target.inference_profile.clone())
        .ok_or("resolved provenance was not persisted")?;
    assert!(
        provenance.starts_with(INFERENCE_PROFILE_PROVENANCE_PREFIX),
        "provenance must be a resolved reference, got {provenance}"
    );
    let encoded = serde_json::to_string(&persisted)?.to_ascii_lowercase();
    for forbidden in ["credential", "authorization", "bearer", "api_key", "secret"] {
        assert!(
            !encoded.contains(forbidden),
            "persisted provenance must be credential-free; found {forbidden}"
        );
    }
    Ok(())
}

#[test]
fn two_node_bindings_bind_two_distinct_revisions() -> Result<(), Box<dyn std::error::Error>> {
    let catalog = baseline_catalog();
    let decide = catalog.resolve("decision.synthetic.v1", "decide")?;
    let planner = catalog.resolve("planner.synthetic.v1", "adaptive_region")?;
    assert_ne!(decide.digest, planner.digest);
    let definition: WorkflowDefinition = decode_strict(&serde_json::to_vec(
        &two_node_definition("decision.synthetic.v1", "planner.synthetic.v1"),
    )?)?;
    let resolved = sts2_harness::management::resolve_definition(&catalog, &definition, None)?;
    assert_eq!(resolved.bindings.len(), 2);
    assert_eq!(resolved.bindings[0].profile_id, "decision.synthetic.v1");
    assert_eq!(resolved.bindings[0].node_kind, "decide");
    assert_eq!(resolved.bindings[1].profile_id, "planner.synthetic.v1");
    assert_eq!(resolved.bindings[1].node_kind, "adaptive_region");
    assert_ne!(resolved.bindings[0].digest, resolved.bindings[1].digest);
    Ok(())
}

#[test]
fn unknown_profile_id_refuses_before_inference() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture(
        CatalogCapabilityDouble::serving(vec![baseline_catalog()]),
        &two_node_definition("decision.missing.v1", "planner.synthetic.v1"),
        "request-unknown",
    )?;
    let error = submit(&fixture)
        .err()
        .ok_or("an unknown profile id unexpectedly admitted")?;
    assert_eq!(error.code, "inference_profile_unknown");
    assert_eq!(fixture.submissions.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn profile_digest_mismatch_refuses_before_inference() -> Result<(), Box<dyn std::error::Error>> {
    let catalog = baseline_catalog();
    let pinned_digest = catalog.descriptors[0].digest.clone();
    let mangled = format!("{}0{}", &pinned_digest[..1], &pinned_digest[2..]);
    assert_ne!(mangled, pinned_digest);
    let fixture = fixture(
        CatalogCapabilityDouble::serving(vec![catalog]),
        &two_node_definition(
            &format!("decision.synthetic.v1:1.0.0:{mangled}"),
            "planner.synthetic.v1",
        ),
        "request-digest",
    )?;
    let error = submit(&fixture)
        .err()
        .ok_or("a pinned digest mismatch unexpectedly admitted")?;
    assert_eq!(error.code, "inference_profile_digest_mismatch");
    assert_eq!(fixture.submissions.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn revoked_profile_refuses_before_inference() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture(
        CatalogCapabilityDouble::serving(vec![catalog_with_decide(
            InferenceProfileState::Revoked,
            true,
        )]),
        &two_node_definition("decision.synthetic.v1", "planner.synthetic.v1"),
        "request-revoked",
    )?;
    let error = submit(&fixture)
        .err()
        .ok_or("a revoked profile unexpectedly admitted")?;
    assert_eq!(error.code, "inference_profile_revoked");
    assert_eq!(fixture.submissions.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn unsupported_model_or_settings_refuse_before_inference() -> Result<(), Box<dyn std::error::Error>>
{
    for state in [
        InferenceProfileState::Unsupported,
        InferenceProfileState::Stale,
        InferenceProfileState::Disabled,
    ] {
        let fixture = fixture(
            CatalogCapabilityDouble::serving(vec![catalog_with_decide(state, true)]),
            &two_node_definition("decision.synthetic.v1", "planner.synthetic.v1"),
            "request-unsupported",
        )?;
        let error = submit(&fixture)
            .err()
            .ok_or("an unadmittable profile unexpectedly admitted")?;
        assert!(
            matches!(
                error.code.as_str(),
                "inference_profile_unsupported"
                    | "inference_profile_stale"
                    | "inference_profile_disabled"
            ),
            "unexpected refusal code {}",
            error.code
        );
        assert_eq!(fixture.submissions.load(Ordering::SeqCst), 0);
    }
    Ok(())
}

#[test]
fn catalog_refresh_causes_zero_inference() -> Result<(), Box<dyn std::error::Error>> {
    let port = Arc::new(MutableCatalogPort {
        catalog: std::sync::Mutex::new(baseline_catalog()),
    });
    let fixture = fixture(
        Arc::new(MutatingCatalogDouble {
            port: Arc::clone(&port) as Arc<dyn LiveInferenceProfileCatalogPort>,
        }),
        &two_node_definition("decision.synthetic.v1", "planner.synthetic.v1"),
        "request-refresh",
    )?;
    let admitted = submit(&fixture);
    assert!(
        admitted.is_ok(),
        "the baseline profile must admit: {admitted:?}"
    );
    let after_admission = fixture.submissions.load(Ordering::SeqCst);

    // Refresh the served catalog. Reading it must reach no provider, and a
    // fresh admission must observe the refreshed revision rather than a cached
    // one.
    *port.catalog.lock().expect("catalog lock") =
        catalog_with_decide(InferenceProfileState::Revoked, true);
    let refreshed = fixture
        .service
        .inference_profile_catalog(&actor().expect("actor"))?;
    refreshed.validate()?;
    assert_eq!(fixture.submissions.load(Ordering::SeqCst), after_admission);

    let refused = fixture.service.submit_run(
        &actor().expect("actor"),
        request_with_identity(&fixture.request, "request-refresh-2"),
    );
    assert_eq!(
        refused.err().map(|error| error.code),
        Some("inference_profile_revoked".to_owned()),
        "the refreshed catalog must refuse the same reference"
    );
    assert_eq!(fixture.submissions.load(Ordering::SeqCst), after_admission);
    Ok(())
}

#[test]
fn select_grant_cannot_edit_protected_configuration() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = fixture(
        CatalogCapabilityDouble::serving(vec![catalog_with_decide(
            InferenceProfileState::Available,
            false,
        )]),
        &two_node_definition("decision.synthetic.v1", "planner.synthetic.v1"),
        "request-select",
    )?;
    let error = submit(&fixture)
        .err()
        .ok_or("a profile without the select grant unexpectedly admitted")?;
    assert_eq!(error.code, "inference_profile_select_denied");
    assert_eq!(fixture.submissions.load(Ordering::SeqCst), 0);

    // `edit` is published, never granted, by this catalog: a caller holding
    // only select authority has no edit path through the catalog surface.
    let catalog = catalog_with_decide(InferenceProfileState::Available, false);
    let descriptor = &catalog.descriptors[0];
    assert!(!descriptor.grants.select);
    assert!(!descriptor.grants.edit);
    Ok(())
}

#[test]
fn catalog_conforms_to_the_versioned_closed_schema() -> Result<(), Box<dyn std::error::Error>> {
    let schema: Value = serde_json::from_slice(include_bytes!(
        "../../../contracts/inference-profile/catalog.schema.json"
    ))?;
    let validator = jsonschema::validator_for(&schema)?;
    let catalog = baseline_catalog();
    catalog.validate()?;
    let encoded = serde_json::to_value(&catalog)?;
    assert!(
        validator.is_valid(&encoded),
        "served catalog does not conform to the closed schema: {:?}",
        validator
            .iter_errors(&encoded)
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
    );
    let mut tampered = encoded.clone();
    tampered["credentials"] = json!("secret");
    assert!(
        !validator.is_valid(&tampered),
        "the closed schema must reject an unadvertised field"
    );
    let mut unversioned = encoded;
    unversioned["descriptors"][0]["version"] = json!("1.0");
    assert!(
        !validator.is_valid(&unversioned),
        "the closed schema must reject a non-semver revision"
    );
    Ok(())
}

#[test]
fn synthetic_fixture_resolves_against_the_synthetic_catalog()
-> Result<(), Box<dyn std::error::Error>> {
    let catalog = sts2_harness::management::synthetic_inference_profile_catalog()?;
    let definition: WorkflowDefinition = decode_strict(WORKFLOW_DEFINITION)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    let resolved = sts2_harness::management::resolve_definition(&catalog, &definition, None)?;
    assert_eq!(resolved.bindings.len(), 1);
    assert_eq!(resolved.bindings[0].profile_id, "decision.synthetic.v1");
    assert_eq!(resolved.bindings[0].requested_model, "synthetic.model.v1");
    assert!(
        resolved
            .reference()
            .starts_with(INFERENCE_PROFILE_PROVENANCE_PREFIX)
    );
    Ok(())
}

#[test]
fn catalog_route_is_read_scoped_and_credential_free() -> Result<(), Box<dyn std::error::Error>> {
    // The catalog is served at `GET /v1/inference-profiles` under `workflow:read`
    // and carries no credential. A reader scope is admitted; an actor with no
    // scope is refused with `missing_scope` and reaches no catalog double.
    let fixture = fixture(
        CatalogCapabilityDouble::serving(vec![baseline_catalog()]),
        &two_node_definition("decision.synthetic.v1", "planner.synthetic.v1"),
        "request-route",
    )?;

    let reader = AuthContext::new("integration.tester", ["workflow:read".to_owned()])?;
    let (status, value) = call_route(
        &fixture.service,
        StaticAuthenticator::single("read-token", reader)?,
        "read-token",
    )?;
    assert_eq!(status, 200, "body: {value}");
    let catalog: sts2_harness::management::InferenceProfileCatalog =
        decode_strict(&serde_json::to_vec(&value)?)?;
    catalog.validate()?;
    assert_eq!(catalog.descriptors.len(), 2);

    let encoded = value.to_string().to_ascii_lowercase();
    for forbidden in ["credential", "authorization", "bearer", "api_key", "secret"] {
        assert!(
            !encoded.contains(forbidden),
            "the served catalog must be credential-free; found {forbidden}"
        );
    }

    let scoped_out = AuthContext::new("integration.tester", [] as [String; 0])?;
    let (status, value) = call_route(
        &fixture.service,
        StaticAuthenticator::single("bare-token", scoped_out)?,
        "bare-token",
    )?;
    assert_eq!(status, 403, "body: {value}");
    assert_eq!(
        value
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("missing_scope")
    );
    Ok(())
}

/// Reads the catalog route over a loopback management server, returning the
/// status and decoded body so a test can assert both the admitted and the
/// scope-refused shape.
fn call_route(
    service: &Arc<ManagementService>,
    authenticator: StaticAuthenticator,
    token: &str,
) -> Result<(u16, Value), Box<dyn std::error::Error>> {
    let config = ServerConfig::new(
        "127.0.0.1:0".parse::<std::net::SocketAddr>()?,
        Arc::new(authenticator),
    )?;
    let server = ManagementServer::start(config, Arc::clone(service))?;
    let client = ManagementClient::new(server.address(), token)?;
    let response = client.request_json("GET", "/v1/inference-profiles", None)?;
    server.shutdown()?;
    Ok((response.status, serde_json::from_slice(&response.body)?))
}
