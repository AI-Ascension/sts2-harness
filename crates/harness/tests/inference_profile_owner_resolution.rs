// SPDX-License-Identifier: MIT

//! Owner-authoritative inference-profile resolution at definition validation
//! and Studio publication (Refs #799).
//!
//! Everything here is synthetic and labelled as such: the catalogs are in-memory
//! owner doubles and the authoring store is in-memory. No provider, model,
//! credential or native host is contacted. A pass here is component evidence that
//! the owner is the single admission authority over inference-profile
//! references. It is not provider-execution evidence, and it says nothing about
//! whether any named profile can actually reach a model.

#![allow(clippy::expect_used)]

use serde_json::json;

#[path = "support/inference_profile_owner_resolution_doubles.rs"]
mod doubles;

use doubles::{
    DECISION_PROFILE, PLANNER_PROFILE, UNCATALOGUED_PROFILE, actor, catalog, definition, publish,
    service, validate,
};

// ---------------------------------------------------------------- property 1

/// A floating reference resolves to an *exact* `profile_id:version:digest`
/// pin, published alongside the reference exactly as authored.
#[test]
fn floating_reference_resolves_to_a_published_exact_pin() -> Result<(), Box<dyn std::error::Error>>
{
    let definition = definition(DECISION_PROFILE, PLANNER_PROFILE);
    let catalog = catalog()?;
    let expected: Vec<(String, String)> = catalog
        .descriptors
        .iter()
        .map(|descriptor| {
            (
                descriptor.profile_id.clone(),
                format!(
                    "{}:{}:{}",
                    descriptor.profile_id, descriptor.version, descriptor.digest
                ),
            )
        })
        .collect();

    let response = validate(&service(Some(catalog))?, &definition)?;

    assert!(response.valid, "definition unexpectedly invalid");
    let published = response
        .inference_profiles
        .ok_or("the owner published no inference-profile decision")?;
    assert_eq!(published.len(), 2, "both references must be published");
    for resolved in &published {
        let (_, expected_pin) = expected
            .iter()
            .find(|(profile_id, _)| profile_id == &resolved.profile_ref)
            .ok_or("a published reference was not authored by this definition")?;
        assert_eq!(
            &resolved.resolved_pin, expected_pin,
            "a floating reference must publish the exact catalog revision"
        );
    }
    Ok(())
}

// ---------------------------------------------------------------- property 2

/// A floating id the catalog does not advertise is refused by the owner, with
/// the catalog's own reason vocabulary. This is the exact case the browser-side
/// gate refused while the owner accepted it.
#[test]
fn uncatalogued_floating_reference_is_refused_by_the_owner()
-> Result<(), Box<dyn std::error::Error>> {
    let definition = definition(DECISION_PROFILE, UNCATALOGUED_PROFILE);
    let service = service(Some(catalog()?))?;

    let error = validate(&service, &definition)
        .err()
        .ok_or("an uncatalogued floating reference unexpectedly validated")?;
    assert_eq!(
        error.code, "inference_profile_unknown",
        "the owner must refuse with its own reason vocabulary"
    );

    let publish_error = publish(&service, &definition)
        .err()
        .ok_or("an uncatalogued floating reference unexpectedly published")?;
    assert_eq!(
        publish_error.code, "inference_profile_unknown",
        "publication must refuse with the same authority as validation"
    );
    Ok(())
}

/// The refusal is fail-closed for publication too: nothing is published when
/// any one reference cannot be resolved.
#[test]
fn publication_refuses_when_only_one_reference_is_uncatalogued()
-> Result<(), Box<dyn std::error::Error>> {
    let definition = definition(UNCATALOGUED_PROFILE, PLANNER_PROFILE);
    let service = service(Some(catalog()?))?;
    let error = publish(&service, &definition)
        .err()
        .ok_or("a single uncatalogued reference unexpectedly published")?;
    assert_eq!(error.code, "inference_profile_unknown");
    let definitions = service.studio_definitions(&actor().expect("actor"))?;
    assert!(
        definitions.definitions.is_empty(),
        "a refused publication must not create a definition"
    );
    Ok(())
}

// ---------------------------------------------------------------- property 3

/// The published pin equals the revision the catalog resolves to, and is not
/// the authored floating id: a consumer recording `resolved_pin` is immune to a
/// later catalog revision, and one recording `profile_ref` is not.
#[test]
fn published_pin_is_the_resolved_revision_not_the_authored_floating_id()
-> Result<(), Box<dyn std::error::Error>> {
    let definition = definition(DECISION_PROFILE, PLANNER_PROFILE);
    let catalog = catalog()?;
    let response = validate(&service(Some(catalog.clone()))?, &definition)?;
    let published = response
        .inference_profiles
        .ok_or("the owner published no inference-profile decision")?;

    for resolved in &published {
        let descriptor = catalog
            .resolve(&resolved.profile_ref, &resolved.node_kind)
            .map_err(|error| {
                format!("the catalog could not re-resolve a published reference: {error}")
            })?;
        assert_eq!(resolved.resolved_pin.split(':').count(), 3);
        assert!(
            resolved.resolved_pin.contains(&descriptor.digest),
            "the published pin must carry the resolved revision digest"
        );
        assert_ne!(
            resolved.resolved_pin, resolved.profile_ref,
            "a floating reference must publish a distinct exact pin"
        );
    }
    Ok(())
}

/// An already-exact authored reference publishes that same exact pin, so an
/// immutability-favouring consumer and the owner agree.
#[test]
fn exact_authored_reference_publishes_its_own_pin() -> Result<(), Box<dyn std::error::Error>> {
    let catalog = catalog()?;
    let descriptor = catalog
        .descriptors
        .iter()
        .find(|descriptor| descriptor.profile_id == DECISION_PROFILE)
        .ok_or("the fixture catalog lost its decision descriptor")?;
    let pinned = format!(
        "{}:{}:{}",
        descriptor.profile_id, descriptor.version, descriptor.digest
    );
    let definition = definition(&pinned, PLANNER_PROFILE);

    let response = validate(&service(Some(catalog))?, &definition)?;
    let published = response
        .inference_profiles
        .ok_or("the owner published no inference-profile decision")?;
    let decide = published
        .iter()
        .find(|resolved| resolved.node_kind == "decide")
        .ok_or("the decide reference was not published")?;
    assert_eq!(decide.profile_ref, pinned);
    assert_eq!(decide.resolved_pin, pinned);
    Ok(())
}

// --------------------------------------------------- shared authority & shape

/// Validation and publication publish the *same* decision for the same
/// document. This is the property that removes the authority asymmetry: a
/// consumer can read either surface and needs no rule of its own.
#[test]
fn validation_and_publication_publish_the_same_owner_decision()
-> Result<(), Box<dyn std::error::Error>> {
    let definition = definition(DECISION_PROFILE, PLANNER_PROFILE);
    let catalog = catalog()?;
    let validated = validate(&service(Some(catalog.clone()))?, &definition)?
        .inference_profiles
        .ok_or("validation published no decision")?;
    let published = publish(&service(Some(catalog))?, &definition)?
        .inference_profiles
        .ok_or("publication published no decision")?;
    assert_eq!(validated, published);
    Ok(())
}

/// Each published reference names the node it came from and its structural
/// path, so a refusal or a review can locate it.
#[test]
fn published_references_carry_their_node_and_path() -> Result<(), Box<dyn std::error::Error>> {
    let definition = definition(DECISION_PROFILE, PLANNER_PROFILE);
    let response = validate(&service(Some(catalog()?))?, &definition)?;
    let published = response
        .inference_profiles
        .ok_or("the owner published no inference-profile decision")?;

    let decide = published
        .iter()
        .find(|resolved| resolved.node_kind == "decide")
        .ok_or("the decide reference was not published")?;
    assert_eq!(decide.graph_id, "main");
    assert_eq!(decide.node_id, "decide");
    assert_eq!(
        decide.path,
        "$.graphs[0].nodes[0].config.decision_profile_ref"
    );

    let plan = published
        .iter()
        .find(|resolved| resolved.node_kind == "adaptive_region")
        .ok_or("the planner reference was not published")?;
    assert_eq!(plan.node_id, "plan");
    assert_eq!(plan.path, "$.graphs[0].nodes[1].config.planner_profile_ref");
    Ok(())
}

/// An owner that serves no catalog publishes no decision rather than an empty
/// one, so a consumer cannot read "no profiles" as "these profiles are fine".
#[test]
fn an_owner_without_a_catalog_publishes_no_decision() -> Result<(), Box<dyn std::error::Error>> {
    let definition = definition(UNCATALOGUED_PROFILE, UNCATALOGUED_PROFILE);
    let response = validate(&service(None)?, &definition)?;
    assert!(
        response.inference_profiles.is_none(),
        "an owner with no catalog must not publish an inference-profile decision"
    );
    Ok(())
}

/// A definition carrying no inference-profile reference publishes an empty
/// decision, which is a different statement from "no authority exercised".
#[test]
fn a_definition_without_references_publishes_an_empty_decision()
-> Result<(), Box<dyn std::error::Error>> {
    let definition = definition(DECISION_PROFILE, PLANNER_PROFILE);
    let mut without = definition;
    let graphs = without["graphs"].as_array_mut().expect("graphs");
    graphs[0]["entry_node"] = json!("observe");
    graphs[0]["nodes"] = json!([{
        "id": "observe",
        "kind": "observe",
        "config": {"projection_ref": "fair-play.synthetic.v1"}
    },
    {"id": "done", "kind": "terminal", "config": {"outcome": "completed"}}]);
    graphs[0]["edges"] = json!([{"from": "observe", "to": "done", "on": "ok", "priority": 0}]);
    let response = validate(&service(Some(catalog()?))?, &without)
        .map_err(|error| format!("{}: {}", error.code, error.message))?;
    assert_eq!(response.inference_profiles, Some(Vec::new()));
    Ok(())
}
