// SPDX-License-Identifier: MIT

//! Admitted inference-profile revision edits: compare-and-swap, immutability
//! and authority (#104 acceptance criterion 3, lane 104-B).
//!
//! Everything here is synthetic and labelled as such: the catalogs are
//! in-memory owner doubles, the execution port only records that it was called,
//! and no provider, model, credential or native host is contacted. A pass is
//! source/component evidence for the edit fences, not provider-execution
//! evidence.
//!
//! The published revision of a profile is the owner's. An accepted edit is
//! recorded in the server-owned journal and reported as the exact
//! `profile_id:version:digest` reference a *new* definition pins, so adoption is
//! definition-scoped by construction.

#![allow(clippy::expect_used)]

use std::sync::Arc;

use serde_json::{Value, json};
use sts2_harness::management::{
    InferenceProfileRevisionJournal, InferenceProfileRevisionRequest,
    InferenceProfileRevisionResponse, ManagementClient, ManagementServer,
    MemoryInferenceProfileRevisionJournal, ServerConfig, StaticAuthenticator, decode_strict,
};

#[path = "support/live_workflow_context_owner.rs"]
mod context_owner_double;
#[path = "support/inference_profile_admission_fixtures.rs"]
mod inference_profile_admission_fixtures;
#[path = "support/inference_profile_catalog_doubles.rs"]
mod inference_profile_catalog_doubles;
#[path = "support/inference_profile_catalog_fixtures.rs"]
mod inference_profile_catalog_fixtures;
#[path = "support/inference_profile_revision_support.rs"]
mod inference_profile_revision_support;

use inference_profile_catalog_doubles::*;
use inference_profile_catalog_fixtures::*;
use inference_profile_revision_support::*;

#[test]
fn concurrent_edits_swap_once_and_the_loser_names_the_winner()
-> Result<(), Box<dyn std::error::Error>> {
    let catalog = editable_catalog();
    let served = catalog.descriptors[0].clone();
    assert!(served.grants.edit, "the fixture publishes the edit grant");
    let journal = Arc::new(MemoryInferenceProfileRevisionJournal::default());
    let fixture = fixture_with_revision_journal(
        CatalogCapabilityDouble::serving(vec![catalog]),
        &two_node_definition(DECIDE, PLANNER),
        "request-cas",
        Arc::clone(&journal) as Arc<dyn InferenceProfileRevisionJournal>,
    )?;

    // Both callers authored their edit against the same served revision, so
    // only the first append may win the swap.
    let winner = fixture.service.adopt_inference_profile_revision(
        &writer()?,
        DECIDE,
        edit(&served.digest, "mutation-first", "1.1.0"),
    )?;
    assert_eq!(winner.outcome, "adopted");
    assert_eq!(winner.profile_id, DECIDE);
    assert_eq!(winner.reference, reference(&winner.revision));
    assert_eq!(winner.revision.version, "1.1.0");
    assert_eq!(winner.revision.adapter, served.adapter);
    assert_eq!(winner.revision.node_kinds, served.node_kinds);
    assert_eq!(winner.revision.grants, served.grants);

    let loser = fixture.service.adopt_inference_profile_revision(
        &writer()?,
        DECIDE,
        edit(&served.digest, "mutation-second", "1.2.0"),
    )?;
    assert_eq!(loser.outcome, "conflict");
    assert_eq!(loser.reference, winner.reference);
    assert_eq!(loser.revision.digest, winner.revision.digest);

    // Immutability: the replaced revision is still held, unrewritten, and the
    // accepted head is the winner rather than the loser.
    let history = journal.history(DECIDE)?;
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].digest, served.digest);
    assert_eq!(history[0].version, served.version);
    assert_eq!(history[1].digest, winner.revision.digest);
    assert_eq!(
        journal.head(DECIDE)?.map(|head| head.digest),
        Some(winner.revision.digest.clone())
    );
    assert!(
        history.iter().all(|held| held.version != "1.2.0"),
        "a lost swap must not be recorded"
    );
    Ok(())
}

#[test]
fn a_retried_mutation_id_replays_and_cannot_smuggle_a_second_edit()
-> Result<(), Box<dyn std::error::Error>> {
    let catalog = editable_catalog();
    let served = catalog.descriptors[0].clone();
    let journal = Arc::new(MemoryInferenceProfileRevisionJournal::default());
    let fixture = fixture_with_revision_journal(
        CatalogCapabilityDouble::serving(vec![catalog]),
        &two_node_definition(DECIDE, PLANNER),
        "request-replay",
        Arc::clone(&journal) as Arc<dyn InferenceProfileRevisionJournal>,
    )?;
    let retry = fixture.service.adopt_inference_profile_revision(
        &writer()?,
        DECIDE,
        edit(&served.digest, "mutation-a", "1.1.0"),
    )?;
    assert_eq!(retry.outcome, "adopted");

    // The identical retry is replayed, not applied twice.
    let replayed = fixture.service.adopt_inference_profile_revision(
        &writer()?,
        DECIDE,
        edit(&served.digest, "mutation-a", "1.1.0"),
    )?;
    assert_eq!(replayed.outcome, "replayed");
    assert_eq!(replayed.reference, retry.reference);
    assert_eq!(journal.history(DECIDE)?.len(), 2);

    // Re-using the identity for a *different* revision is a conflict rather
    // than a silent overwrite of the accepted revision.
    let reused = fixture.service.adopt_inference_profile_revision(
        &writer()?,
        DECIDE,
        edit(&served.digest, "mutation-a", "1.3.0"),
    )?;
    assert_eq!(reused.outcome, "conflict");
    assert_eq!(reused.reference, retry.reference);
    assert_eq!(journal.history(DECIDE)?.len(), 2);
    Ok(())
}

#[test]
fn read_and_select_grants_confer_no_edit_authority() -> Result<(), Box<dyn std::error::Error>> {
    let editable = editable_catalog();
    let served = editable.descriptors[0].clone();
    let journal = Arc::new(MemoryInferenceProfileRevisionJournal::default());
    let fixture = fixture_with_revision_journal(
        CatalogCapabilityDouble::serving(vec![editable.clone()]),
        &two_node_definition(DECIDE, PLANNER),
        "request-authority",
        Arc::clone(&journal) as Arc<dyn InferenceProfileRevisionJournal>,
    )?;

    // A reader scope discovers the catalog but cannot edit through it.
    let refused = fixture.service.adopt_inference_profile_revision(
        &actor(&["workflow:read"])?,
        DECIDE,
        edit(&served.digest, "mutation-read", "1.1.0"),
    );
    assert_eq!(
        refused.err().map(|error| error.code),
        Some("missing_scope".to_owned())
    );

    // The planner is discoverable and selectable, and its owner publishes no
    // edit grant, so the write scope alone is not sufficient.
    let planner = editable.descriptors[1].clone();
    assert!(planner.grants.select && !planner.grants.edit);
    let denied = fixture.service.adopt_inference_profile_revision(
        &writer()?,
        PLANNER,
        edit(&planner.digest, "mutation-planner", "1.1.0"),
    );
    assert_eq!(
        denied.err().map(|error| error.code),
        Some("inference_profile_edit_denied".to_owned())
    );
    assert!(journal.history(DECIDE)?.is_empty());
    assert!(journal.history(PLANNER)?.is_empty());

    // A service composed without a journal refuses the edit rather than
    // accepting it into a process-local map an operator cannot see.
    let unjournaled = fixture_selecting(
        CatalogCapabilityDouble::serving(vec![editable_catalog()]),
        &two_node_definition(DECIDE, PLANNER),
        "request-no-journal",
        None,
    )?;
    let unavailable = unjournaled.service.adopt_inference_profile_revision(
        &writer()?,
        DECIDE,
        edit(&served.digest, "mutation-unjournaled", "1.1.0"),
    );
    assert_eq!(
        unavailable.err().map(|error| error.code),
        Some("inference_profile_revision_journal_unavailable".to_owned())
    );
    Ok(())
}

#[test]
fn a_stale_expectation_is_refused_before_the_swap() -> Result<(), Box<dyn std::error::Error>> {
    let catalog = editable_catalog();
    let served = catalog.descriptors[0].clone();
    let planner = catalog.descriptors[1].clone();
    let journal = Arc::new(MemoryInferenceProfileRevisionJournal::default());
    let fixture = fixture_with_revision_journal(
        CatalogCapabilityDouble::serving(vec![catalog]),
        &two_node_definition(DECIDE, PLANNER),
        "request-stale",
        Arc::clone(&journal) as Arc<dyn InferenceProfileRevisionJournal>,
    )?;

    // A digest that names a different published revision is not the revision
    // being edited.
    let stale = fixture.service.adopt_inference_profile_revision(
        &writer()?,
        DECIDE,
        edit(&planner.digest, "mutation-stale", "1.1.0"),
    );
    assert_eq!(
        stale.err().map(|error| error.code),
        Some("inference_profile_revision_conflict".to_owned())
    );

    // An id the owner does not advertise cannot be edited either.
    let unknown = fixture.service.adopt_inference_profile_revision(
        &writer()?,
        "decision.missing.v1",
        edit(&served.digest, "mutation-unknown", "1.1.0"),
    );
    assert_eq!(
        unknown.err().map(|error| error.code),
        Some("inference_profile_unknown".to_owned())
    );

    // Neither refusal reached the journal.
    assert!(journal.history(DECIDE)?.is_empty());
    assert!(journal.head(DECIDE)?.is_none());

    // A restated identity is not a new revision, so it is refused rather than
    // re-sealed under the version the catalog already keys.
    let unchanged = fixture.service.adopt_inference_profile_revision(
        &writer()?,
        DECIDE,
        edit(&served.digest, "mutation-unchanged", &served.version),
    );
    assert_eq!(
        unchanged.err().map(|error| error.code),
        Some("inference_profile_revision_version_unchanged".to_owned())
    );
    assert!(journal.history(DECIDE)?.is_empty());
    Ok(())
}

#[test]
fn the_edit_body_is_closed_and_carries_no_authority_or_credential_field()
-> Result<(), Box<dyn std::error::Error>> {
    let request = edit("00", "mutation-shape", "1.1.0");
    let encoded = serde_json::to_value(&request)?;
    let mut fields = encoded
        .as_object()
        .ok_or("the request must encode as an object")?
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    fields.sort();
    assert_eq!(
        fields,
        vec![
            "client_mutation_id",
            "effective_budgets",
            "expected_revision_digest",
            "prompt_revision",
            "schema_version",
            "settings_revision",
            "supported_settings",
            "version",
        ]
    );

    // Every unadvertised field is rejected, including the ones that would
    // otherwise restate an identity or an authority the edit must inherit.
    for forbidden in [
        json!({"credentials": "secret"}),
        json!({"endpoint": "https://example.invalid"}),
        json!({"executable": "provider.exe"}),
        json!({"adapter": "other.provider.v1"}),
        json!({"requested_model": "other.model.v1"}),
        json!({"grants": {"select": true, "edit": true}}),
        json!({"state": "available"}),
        json!({"node_kinds": ["decide"]}),
        json!({"operations": ["decide"]}),
    ] {
        let mut tampered = encoded.clone();
        let object = tampered
            .as_object_mut()
            .ok_or("the request must encode as an object")?;
        for (key, value) in forbidden
            .as_object()
            .ok_or("each probe must be an object")?
        {
            object.insert(key.clone(), value.clone());
        }
        let bytes = serde_json::to_vec(&tampered)?;
        assert!(
            decode_strict::<InferenceProfileRevisionRequest>(&bytes).is_err(),
            "the closed request must reject {forbidden}"
        );
    }
    Ok(())
}

#[test]
fn the_edit_route_serves_the_response_and_refuses_every_other_shape()
-> Result<(), Box<dyn std::error::Error>> {
    let catalog = editable_catalog();
    let served = catalog.descriptors[0].clone();
    let fixture = fixture_with_revision_journal(
        CatalogCapabilityDouble::serving(vec![catalog]),
        &two_node_definition(DECIDE, PLANNER),
        "request-route",
        Arc::new(MemoryInferenceProfileRevisionJournal::default()),
    )?;
    let config = ServerConfig::new(
        "127.0.0.1:0".parse::<std::net::SocketAddr>()?,
        Arc::new(StaticAuthenticator::single("writer-token", writer()?)?),
    )?;
    let server = ManagementServer::start(config, Arc::clone(&fixture.service))?;
    let client = ManagementClient::new(server.address(), "writer-token")?;
    let path = format!("/v1/inference-profiles/{DECIDE}/revisions");
    let body = serde_json::to_vec(&edit(&served.digest, "mutation-route", "1.1.0"))?;

    let response = client.request_json("POST", &path, Some(&body))?;
    assert_eq!(response.status, 200, "body: {:?}", response.body);
    let admitted: InferenceProfileRevisionResponse =
        decode_strict(&response.body).map_err(|error| std::io::Error::other(error.to_string()))?;
    assert_eq!(admitted.outcome, "adopted");
    assert_eq!(admitted.revision.version, "1.1.0");

    // The adopted revision is reported to a reader without any credential.
    let encoded = String::from_utf8(response.body.clone())?.to_ascii_lowercase();
    for forbidden in ["credential", "authorization", "bearer", "api_key", "secret"] {
        assert!(
            !encoded.contains(forbidden),
            "the edit response must be credential-free; found {forbidden}"
        );
    }

    // Nothing else lives under this prefix: a GET, a query string, a deeper
    // sub-resource and a wrong method are all refused as not found. The catalog
    // route itself is unchanged and takes no POST.
    for (method, candidate, body) in [
        ("GET", path.as_str(), None),
        ("POST", "/v1/inference-profiles", Some(body.as_slice())),
        (
            "POST",
            "/v1/inference-profiles/decision.synthetic.v1/revisions/1",
            Some(body.as_slice()),
        ),
        ("PUT", path.as_str(), Some(body.as_slice())),
        (
            "POST",
            "/v1/inference-profiles/decision.synthetic.v1/revisions?force=1",
            Some(body.as_slice()),
        ),
    ] {
        let response = client.request_json(method, candidate, body)?;
        assert_ne!(response.status, 200, "{method} {candidate} was admitted");
        let value: Value = serde_json::from_slice(&response.body)?;
        assert_eq!(
            value
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("route_not_found"),
            "{method} {candidate} body: {value}"
        );
    }
    server.shutdown()?;
    Ok(())
}
