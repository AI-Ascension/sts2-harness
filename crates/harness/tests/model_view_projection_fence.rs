// SPDX-License-Identifier: MIT

//! Excluded sentinels, source immutability, and approval fencing (issue #110).
//!
//! Synthetic fixtures only; no provider, host, or game is contacted.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

#[path = "support/model_view_projection.rs"]
mod fixture;

use fixture::{minimal_recipe, named, observation, path};
use serde_json::{Value, json};
use sts2_harness::context_control::{
    AdmittedSourceObservation, ModelViewApproval, ModelViewProjection, ModelViewProjectionError,
    ModelViewSelectorRegistry, project_model_view, reject_excluded_sentinels,
};

// ---------------------------------------------------------------------------
// AC3 — excluded sentinel fields are absent from prepared bytes while the full
//       owner observation and legal catalog remain unchanged.
// ---------------------------------------------------------------------------

#[test]
fn excluded_sentinels_are_absent_and_the_owner_source_is_unchanged() {
    let recipe = minimal_recipe();
    let original = observation();
    let source = AdmittedSourceObservation::admit(original.clone()).expect("source admits");
    let prepared = project_model_view(&recipe, &source).expect("projection");

    let bytes = String::from_utf8(prepared.bytes.clone()).expect("projected bytes are utf-8");
    for sentinel in ["deck", "discard", "exhaust", "legal_actions", "action_id"] {
        assert!(
            !bytes.contains(sentinel),
            "{sentinel} must be absent from prepared model-view bytes"
        );
    }
    assert!(
        bytes.contains("strike"),
        "a declared hand card must still be present"
    );

    assert_eq!(
        source.as_value(),
        &original,
        "the owner's admitted observation must be byte-identical after projection"
    );
    assert_eq!(
        source
            .as_value()
            .pointer("/legal_actions")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(2),
        "the owner's legal-action catalog must remain complete"
    );
    assert_eq!(
        source.as_value().pointer("/player/deck"),
        original.pointer("/player/deck"),
        "the owner's unseen draw pile must remain unchanged"
    );
}

#[test]
fn a_surviving_sentinel_is_a_hard_refusal() {
    let leaked = json!({"state_id": "s", "player": {"deck": []}});
    assert_eq!(
        reject_excluded_sentinels(&leaked),
        Err(ModelViewProjectionError::ExcludedSentinelPresent {
            path: "player.deck".to_owned()
        }),
        "the sentinel sweep must fail closed rather than trust the walk"
    );
    assert!(reject_excluded_sentinels(&json!({"state_id": "s"})).is_ok());
}

// ---------------------------------------------------------------------------
// AC4 — selector/source/transform changes fence earlier approvals, and
//       seed-visible vs seed-blind regressions remain distinct.
// ---------------------------------------------------------------------------

#[test]
fn changing_the_selector_or_source_fences_an_earlier_approval() {
    let recipe = minimal_recipe();
    let source = AdmittedSourceObservation::admit(observation()).expect("source admits");
    let prepared = project_model_view(&recipe, &source).expect("projection");
    let approval = ModelViewApproval::mint(&recipe, &prepared);
    assert_eq!(approval.verify(&recipe, &prepared), Ok(()));

    // A transform/selector revision change mints a different recipe digest.
    let mut changed = observation();
    changed["visible_seed"] = json!("seed-xyz");
    let changed_source = AdmittedSourceObservation::admit(changed).expect("changed source admits");
    let changed_prepared = project_model_view(&recipe, &changed_source).expect("projection");
    assert_eq!(
        approval.verify(&recipe, &changed_prepared),
        Err(ModelViewProjectionError::ApprovalFenced),
        "a changed source must fence the earlier approval"
    );

    let revised = ModelViewProjection::new("selector-basic", "revision-2", recipe.fields.clone())
        .expect("revised recipe resolves");
    assert_eq!(
        approval.verify(&revised, &prepared),
        Err(ModelViewProjectionError::ApprovalFenced),
        "a changed recipe revision must fence the earlier approval"
    );
    assert_ne!(
        recipe.digest().expect("digest"),
        revised.digest().expect("digest"),
        "a revision change must produce a different recipe digest"
    );
}

#[test]
fn seed_visible_and_seed_blind_projections_are_distinct() {
    let with_seed = ModelViewProjection::new(
        "selector-seed-visible",
        "revision-1",
        vec![
            path(&[named("state_id")]),
            path(&[named("generation")]),
            path(&[named("visible_seed")]),
            path(&[named("player"), named("hp")]),
            path(&[named("player"), named("max_hp")]),
            path(&[named("player"), named("energy")]),
            path(&[named("player"), named("gold")]),
            path(&[named("player"), named("hand")]),
            path(&[named("state"), named("state")]),
        ],
    )
    .expect("seed-visible recipe resolves");
    let seed_blind = ModelViewProjection::new(
        "selector-seed-blind",
        "revision-1",
        vec![
            path(&[named("state_id")]),
            path(&[named("generation")]),
            path(&[named("player"), named("hp")]),
            path(&[named("player"), named("max_hp")]),
            path(&[named("player"), named("energy")]),
            path(&[named("player"), named("gold")]),
            path(&[named("player"), named("hand")]),
            path(&[named("state"), named("state")]),
        ],
    )
    .expect("seed-blind recipe resolves");

    let source = AdmittedSourceObservation::admit(observation()).expect("source admits");
    let visible = project_model_view(&with_seed, &source).expect("seed-visible projection");
    let blind = project_model_view(&seed_blind, &source).expect("seed-blind projection");

    assert_eq!(
        visible.value.get("visible_seed"),
        Some(&json!("seed-abc")),
        "a seed-visible recipe carries the host seed"
    );
    assert!(
        blind.value.get("visible_seed").is_none(),
        "a seed-blind recipe must not carry the host seed"
    );
    assert_ne!(
        visible.projected_digest, blind.projected_digest,
        "the two regressions must be distinguishable by digest"
    );
}

#[test]
fn a_selector_may_not_be_rebound_to_different_fields() {
    let mut registry = ModelViewSelectorRegistry::new();
    let recipe = minimal_recipe();
    let digest = recipe.digest().expect("digest");
    assert_eq!(registry.admit(&recipe.selector_id, &digest), Ok(()));
    assert_eq!(
        registry.admit(&recipe.selector_id, &digest),
        Ok(()),
        "re-registering the identical revision is idempotent"
    );
    assert_eq!(
        registry.admit(&recipe.selector_id, "deadbeef"),
        Err(ModelViewProjectionError::RepeatedSelector {
            selector_id: "selector-basic".to_owned()
        }),
        "one selector identity may not silently change fields"
    );
    assert_eq!(registry.digest_of("selector-basic"), Some(digest.as_str()));
    assert_eq!(registry.selector_ids(), vec!["selector-basic"]);
}

#[test]
fn forged_bytes_are_fenced_by_the_approval() {
    // `PreparedModelView` has public fields and no constructor, so a consumer can pair an honest
    // value with bytes it edited. The bytes are what a downstream caller would actually send, so the
    // approval must bind them too: otherwise excluded content could ride in under a valid approval.
    let recipe = minimal_recipe();
    let source = AdmittedSourceObservation::admit(observation()).expect("source admits");
    let prepared = project_model_view(&recipe, &source).expect("projection");
    let approval = ModelViewApproval::mint(&recipe, &prepared);
    assert_eq!(approval.verify(&recipe, &prepared), Ok(()));

    let mut forged = prepared.clone();
    forged.bytes =
        serde_json::to_vec(&json!({"legal_actions": [{"action_id": "a1"}]})).expect("encode");
    assert_eq!(
        approval.verify(&recipe, &forged),
        Err(ModelViewProjectionError::ApprovalFenced),
        "bytes that do not match the approved digest must fence the approval"
    );
    assert!(
        String::from_utf8(forged.bytes.clone())
            .expect("utf-8")
            .contains("legal_actions"),
        "the forged payload really does carry excluded content"
    );

    // A one-byte edit is enough; the binding is over the exact bytes.
    let mut nudged = prepared.clone();
    nudged.bytes.push(b' ');
    assert_eq!(
        approval.verify(&recipe, &nudged),
        Err(ModelViewProjectionError::ApprovalFenced),
        "even an appended byte must fence the approval"
    );
}
