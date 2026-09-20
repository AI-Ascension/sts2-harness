// SPDX-License-Identifier: MIT

//! Records a restored history must refuse.
//!
//! Restoration re-derives the rules the append path applies, so a record that could not have been
//! appended is refused rather than loaded. Each test here edits one field of an encoded document
//! and checks the exact reason the boundary gives for refusing it.

#![allow(clippy::expect_used, dead_code)]

use sts2_harness::semantic_history::{
    SemanticHistoryCausalParent, SemanticHistoryError, SemanticHistoryKind, SemanticHistoryStore,
};

#[path = "support/semantic_history_fixture.rs"]
mod fixture;

#[test]
fn an_undeclared_jump_is_refused_on_restore() {
    let mut store = SemanticHistoryStore::open(
        fixture::binding(),
        fixture::ROOT,
        fixture::window_with_gap(1, 2, 3),
    )
    .expect("store opens");
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    store
        .append(
            fixture::ROOT,
            fixture::gap_event("event_2", 2),
            SemanticHistoryCausalParent::NotStated,
        )
        .expect("a declared gap event is admitted");
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_3",
        SemanticHistoryKind::CardPlayed,
        4,
    );
    let mut doc = fixture::document(&store);
    // The jump from 2 to 4 was admitted because the gap declared it; without the gap it is a jump
    // the store would never have written.
    doc["window"]["intervals"] = serde_json::json!([]);
    assert_eq!(fixture::refused(&doc), SemanticHistoryError::Sequence);
}

#[test]
fn records_whose_epochs_move_backwards_are_refused() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        5,
    );
    store.advance_epoch(2).expect("advance");
    let mut restarted = fixture::event("event_2", SemanticHistoryKind::CardPlayed, 1, None);
    restarted.authority_epoch = 2;
    store
        .append(
            fixture::ROOT,
            restarted,
            SemanticHistoryCausalParent::NotStated,
        )
        .expect("an event in the new epoch is admitted");
    let mut doc = fixture::document(&store);
    let first = doc["branches"][0]["events"][0].clone();
    let second = doc["branches"][0]["events"][1].clone();
    doc["branches"][0]["events"][0] = second;
    doc["branches"][0]["events"][1] = first;
    assert_eq!(fixture::refused(&doc), SemanticHistoryError::Epoch);
}

#[test]
fn a_record_that_claims_an_epoch_the_store_never_reached_is_refused() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    let mut doc = fixture::document(&store);
    doc["branches"][0]["events"][0]["input"]["authority_epoch"] = serde_json::json!(5);
    assert_eq!(fixture::refused(&doc), SemanticHistoryError::Epoch);
}

#[test]
fn a_repeated_identity_in_a_document_is_refused() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_2",
        SemanticHistoryKind::CardPlayed,
        2,
    );
    let mut doc = fixture::document(&store);
    let first = doc["branches"][0]["events"][0].clone();
    doc["branches"][0]["events"]
        .as_array_mut()
        .expect("events")
        .push(first);
    assert_eq!(
        fixture::refused(&doc),
        SemanticHistoryError::MixedGeneration
    );
}

#[test]
fn a_record_whose_stated_parent_is_absent_is_refused() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    fixture::append_linked(
        &mut store,
        fixture::ROOT,
        "event_2",
        SemanticHistoryKind::ChoiceMade,
        2,
        "event_1",
    );
    let mut doc = fixture::document(&store);
    // The child now precedes the event it names, so its stated cause is not in the history yet.
    let first = doc["branches"][0]["events"][0].clone();
    let second = doc["branches"][0]["events"][1].clone();
    doc["branches"][0]["events"][0] = second;
    doc["branches"][0]["events"][1] = first;
    assert_eq!(fixture::refused(&doc), SemanticHistoryError::Causality);
}

#[test]
fn a_record_whose_digest_does_not_match_its_content_is_refused() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    let mut doc = fixture::document(&store);
    doc["branches"][0]["events"][0]["content_digest"] = serde_json::json!("0".repeat(64));
    assert_eq!(fixture::refused(&doc), SemanticHistoryError::Corrupt);
}

#[test]
fn a_record_edited_without_recomputing_its_digest_is_refused() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    let mut doc = fixture::document(&store);
    // The episode is unconstrained by the kind rules, so only the digest can notice this edit.
    doc["branches"][0]["events"][0]["input"]["episode"] = serde_json::json!(9);
    assert_eq!(fixture::refused(&doc), SemanticHistoryError::Corrupt);
}

#[test]
fn records_whose_order_the_store_could_not_have_written_are_refused() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_2",
        SemanticHistoryKind::CardPlayed,
        2,
    );
    let mut doc = fixture::document(&store);
    let first = doc["branches"][0]["events"][0].clone();
    let second = doc["branches"][0]["events"][1].clone();
    doc["branches"][0]["events"][0] = second;
    doc["branches"][0]["events"][1] = first;
    assert_eq!(fixture::refused(&doc), SemanticHistoryError::Sequence);
}

#[test]
fn a_record_that_mixes_the_causal_arms_is_refused() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    let mut doc = fixture::document(&store);
    doc["branches"][0]["events"][0]["causal_parent"] = serde_json::json!({"state": "stated"});
    assert_eq!(fixture::refused(&doc), SemanticHistoryError::Corrupt);
}

#[test]
fn a_captured_record_inside_a_declared_gap_is_refused_on_restore() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    let mut doc = fixture::document(&store);
    // Declare the recorded sequence a dropped span: the record now claims an observation where the
    // capture says it could not observe one.
    doc["window"]["intervals"] = serde_json::json!([{
        "from_sequence": 1,
        "to_sequence": 1,
        "status": "dropped",
        "label": "capture_dropped"
    }]);
    assert_eq!(fixture::refused(&doc), SemanticHistoryError::Coverage);
}

#[test]
fn a_subject_that_aliases_the_branch_is_refused_on_restore() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    let mut doc = fixture::document(&store);
    doc["branches"][0]["events"][0]["input"]["subjects"][0]["identity"] =
        serde_json::json!(fixture::ROOT);
    assert_eq!(
        fixture::refused(&doc),
        SemanticHistoryError::IdentityNamespaceCollision("actor")
    );
}

#[test]
fn a_record_captured_before_the_window_opens_is_refused_on_restore() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    let mut doc = fixture::document(&store);
    // The capture only claims to have begun at sequence 5, so a record at sequence 1 was not taken
    // by this capture however well formed it is.
    doc["window"]["capture_start"] = serde_json::json!(5);
    assert_eq!(fixture::refused(&doc), SemanticHistoryError::Coverage);
}

#[test]
fn a_gap_record_whose_coverage_disagrees_with_its_interval_is_refused_on_restore() {
    let mut store = SemanticHistoryStore::open(
        fixture::binding(),
        fixture::ROOT,
        fixture::window_with_gap(1, 2, 2),
    )
    .expect("store opens");
    store
        .append(
            fixture::ROOT,
            fixture::gap_event("event_1", 2),
            SemanticHistoryCausalParent::NotStated,
        )
        .expect("a disclosed gap is admitted");
    let mut doc = fixture::document(&store);
    // The span now says it could not be expressed at all, while the record inside it says it was
    // dropped: the two accounts of the same span no longer agree.
    doc["window"]["intervals"][0]["status"] = serde_json::json!("unsupported");
    assert_eq!(fixture::refused(&doc), SemanticHistoryError::Coverage);
}

#[test]
fn an_imported_record_that_states_a_parent_is_refused_on_restore() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    fixture::append_linked(
        &mut store,
        fixture::ROOT,
        "event_2",
        SemanticHistoryKind::ChoiceMade,
        2,
        "event_1",
    );
    let mut doc = fixture::document(&store);
    // An imported event's causality was settled when it was captured, so a parent re-derived at
    // import time is exactly the snapshot inference this boundary refuses.
    doc["branches"][0]["events"][1]["input"]["origin"] = serde_json::json!("imported");
    assert_eq!(
        fixture::refused(&doc),
        SemanticHistoryError::ImportedStatesParent
    );
}

#[test]
fn a_stated_parent_that_does_not_precede_its_child_is_refused_on_restore() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        5,
    );
    store.advance_epoch(2).expect("advance");
    let mut link = fixture::event("event_2", SemanticHistoryKind::ChoiceMade, 1, None);
    link.authority_epoch = 2;
    store
        .append(fixture::ROOT, link, SemanticHistoryCausalParent::NotStated)
        .expect("an event in the new epoch is admitted");
    let mut doc = fixture::document(&store);
    // The new epoch restarts host sequencing, and inside a restarted sequence the parent's number is
    // the larger one: a cause that follows its effect is refused even though the numbers alone would
    // not say so.
    doc["branches"][0]["events"][1]["causal_parent"] =
        serde_json::json!({"state": "stated", "event_id": "event_1"});
    assert_eq!(
        fixture::refused(&doc),
        SemanticHistoryError::ParentNotPreceding
    );
}

#[test]
fn a_stated_parent_in_another_epoch_is_refused_on_restore() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    store.advance_epoch(2).expect("advance");
    let mut link = fixture::event("event_2", SemanticHistoryKind::ChoiceMade, 5, None);
    link.authority_epoch = 2;
    store
        .append(fixture::ROOT, link, SemanticHistoryCausalParent::NotStated)
        .expect("an event in the new epoch is admitted");
    let mut doc = fixture::document(&store);
    // Sequencing restarts with the epoch, so the numbers cannot order these two records against each
    // other; a cause recorded under another epoch is refused rather than compared by number.
    doc["branches"][0]["events"][1]["causal_parent"] =
        serde_json::json!({"state": "stated", "event_id": "event_1"});
    assert_eq!(fixture::refused(&doc), SemanticHistoryError::Causality);
}
