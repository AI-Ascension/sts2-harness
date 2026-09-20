// SPDX-License-Identifier: MIT

//! The encoded document's own shape.
//!
//! One document holds one owner scope, one capture window, one lineage and every branch. A document
//! that does not decode as that shape, or that states an owner scope this boundary would not have
//! written, is refused before any record in it is considered.

#![allow(clippy::expect_used, dead_code)]

use sts2_harness::semantic_history::{SemanticHistoryError, SemanticHistoryKind};

#[path = "support/semantic_history_fixture.rs"]
mod fixture;

#[test]
fn an_unknown_field_in_a_record_is_refused() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    let mut doc = fixture::document(&store);
    doc["branches"][0]["events"][0]["input"]["invented"] = serde_json::json!(true);
    assert_eq!(fixture::refused(&doc), SemanticHistoryError::Corrupt);
}

#[test]
fn a_document_written_under_another_schema_is_refused() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    let mut doc = fixture::document(&store);
    doc["schema"] = serde_json::json!("ascension.semantic-history.v2");
    assert_eq!(fixture::refused(&doc), SemanticHistoryError::Corrupt);
}

#[test]
fn an_unknown_field_in_the_document_is_refused() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    let mut doc = fixture::document(&store);
    doc["invented"] = serde_json::json!(true);
    assert_eq!(fixture::refused(&doc), SemanticHistoryError::Corrupt);
}

#[test]
fn an_unknown_field_in_a_branch_is_refused() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    let mut doc = fixture::document(&store);
    doc["branches"][0]["invented"] = serde_json::json!(true);
    assert_eq!(fixture::refused(&doc), SemanticHistoryError::Corrupt);
}

#[test]
fn a_record_filed_under_another_branch_is_refused_on_restore() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    let mut doc = fixture::document(&store);
    // The branch keeps its identity, so only the record's own claim about where it was written can
    // reveal that it was filed under a branch it does not belong to.
    doc["branches"][0]["events"][0]["branch_id"] = serde_json::json!("branch_other");
    assert_eq!(fixture::refused(&doc), SemanticHistoryError::Corrupt);
}

#[test]
fn a_record_written_under_another_schema_is_refused_on_restore() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    let mut doc = fixture::document(&store);
    doc["branches"][0]["events"][0]["schema"] = serde_json::json!("ascension.semantic-history.v2");
    assert_eq!(fixture::refused(&doc), SemanticHistoryError::Corrupt);
}

#[test]
fn a_binding_that_is_not_an_opaque_identity_is_refused_on_restore() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    let mut doc = fixture::document(&store);
    // An owner scope read as a host path could turn a restored history into a host read.
    doc["binding"]["run_id"] = serde_json::json!("/etc/passwd");
    assert_eq!(
        fixture::refused(&doc),
        SemanticHistoryError::NonOpaqueIdentity("binding.run_id")
    );
}

#[test]
fn a_window_whose_declared_gaps_overlap_is_refused_on_restore() {
    let store = fixture::store();
    let mut doc = fixture::document(&store);
    // Two spans that overlap cannot both be the account of what was dropped.
    doc["window"]["intervals"] = serde_json::json!([
        {
            "from_sequence": 1,
            "to_sequence": 3,
            "status": "dropped",
            "label": "capture_dropped"
        },
        {
            "from_sequence": 3,
            "to_sequence": 5,
            "status": "dropped",
            "label": "capture_dropped"
        }
    ]);
    assert_eq!(fixture::refused(&doc), SemanticHistoryError::Coverage);
}

#[test]
fn a_document_that_names_one_branch_twice_is_refused() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    let mut doc = fixture::document(&store);
    let branch = doc["branches"][0].clone();
    // One branch identity, two histories: the second would overwrite the first rather than be
    // ordered against it.
    doc["branches"] = serde_json::json!([branch.clone(), branch]);
    assert_eq!(fixture::refused(&doc), SemanticHistoryError::Branch);
}
