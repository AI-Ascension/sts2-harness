// SPDX-License-Identifier: MIT

//! The boundary an imported batch is admitted through.
//!
//! The bytes a port hands back are the whole of what the harness gets, so a batch that names
//! another scope, another epoch, a branch this store does not hold, an unknown member, another
//! schema, a non-opaque identity or more of anything than the bound allows is refused here rather
//! than read as history.

#![allow(clippy::expect_used, dead_code)]

use sts2_harness::semantic_history::{
    MAX_HISTORY_IMPORT_BYTES, SemanticHistoryError, SemanticHistoryKind, import_saved_history,
};

#[path = "support/semantic_history_fixture.rs"]
mod fixture;

#[path = "support/semantic_history_import_doubles.rs"]
mod import_doubles;

use import_doubles::*;

#[test]
fn saved_history_from_another_run_or_epoch_is_refused() {
    let mut store = fixture::store();
    let saved = batch("capture_0001", fixture::ROOT, 1);
    let other_run = Saved::new(fixture::binding_other_run()).holding("batch_0001", &saved);
    assert_eq!(
        import_saved_history(&mut store, &other_run).expect_err("another run is refused"),
        SemanticHistoryError::Scope
    );
    let other_epoch = Saved::new(fixture::binding_other_epoch()).holding("batch_0001", &saved);
    assert_eq!(
        import_saved_history(&mut store, &other_epoch).expect_err("another epoch is refused"),
        SemanticHistoryError::Epoch
    );
    assert_eq!(store.len(fixture::ROOT).expect("len"), 0);
}

#[test]
fn a_batch_that_names_a_branch_this_store_does_not_hold_is_refused() {
    let mut store = fixture::store();
    assert_eq!(
        refused(&mut store, &batch("capture_0001", "branch_other", 1)),
        SemanticHistoryError::Branch
    );
    // A batch that carries no events is refused on its branch too: an import that named a scope
    // this store does not hold would otherwise report a successful backfill of nothing.
    assert_eq!(
        refused(&mut store, &batch("capture_0001", "branch_other", 0)),
        SemanticHistoryError::Branch
    );
}

#[test]
fn a_branch_identity_that_is_not_opaque_is_refused() {
    let mut store = fixture::store();
    // A branch identity that could be read as a host path is refused as that, rather than reported
    // as merely unknown, because the reason is part of the closed vocabulary a caller reads.
    assert_eq!(
        refused(&mut store, &batch("capture_0001", "/etc/passwd", 1)),
        SemanticHistoryError::NonOpaqueIdentity("import.branch_id")
    );
}

#[test]
fn a_batch_whose_events_do_not_advance_is_refused() {
    let mut store = fixture::store();
    let mut saved = batch("capture_0001", fixture::ROOT, 2);
    saved.events.reverse();
    assert_eq!(refused(&mut store, &saved), SemanticHistoryError::Sequence);
    assert_eq!(store.len(fixture::ROOT).expect("len"), 0);
}

#[test]
fn an_import_batch_under_another_schema_is_refused() {
    let mut store = fixture::store();
    let saved = batch("capture_0001", fixture::ROOT, 1);
    let mut doc = document(&saved);
    doc["schema"] = serde_json::json!("ascension.semantic-history.v2");
    let port = Saved::new(fixture::binding())
        .holding_bytes("batch_0001", serde_json::to_vec(&doc).expect("bytes"));
    assert_eq!(
        import_saved_history(&mut store, &port).expect_err("another schema is refused"),
        SemanticHistoryError::Corrupt
    );
}

#[test]
fn an_unknown_field_in_the_import_batch_document_is_refused() {
    let mut store = fixture::store();
    let saved = batch("capture_0001", fixture::ROOT, 1);
    for member in ["window", "binding", "authority_epoch"] {
        let mut doc = document(&saved);
        // A batch that could state its own window, scope or epoch could declare its own gaps and
        // close them, or carry history across a scope it was not saved under.
        doc[member] = serde_json::json!(1);
        let port = Saved::new(fixture::binding())
            .holding_bytes("batch_0001", serde_json::to_vec(&doc).expect("bytes"));
        assert_eq!(
            import_saved_history(&mut store, &port).expect_err("the member is refused"),
            SemanticHistoryError::Corrupt
        );
    }
    let mut doc = document(&saved);
    doc["invented"] = serde_json::json!(true);
    let port = Saved::new(fixture::binding())
        .holding_bytes("batch_0001", serde_json::to_vec(&doc).expect("bytes"));
    assert_eq!(
        import_saved_history(&mut store, &port).expect_err("an unknown member is refused"),
        SemanticHistoryError::Corrupt
    );
}

#[test]
fn an_unknown_field_in_an_import_event_is_refused() {
    let mut store = fixture::store();
    let saved = batch("capture_0001", fixture::ROOT, 1);
    let mut doc = document(&saved);
    doc["events"][0]["invented"] = serde_json::json!(true);
    let port = Saved::new(fixture::binding())
        .holding_bytes("batch_0001", serde_json::to_vec(&doc).expect("bytes"));
    assert_eq!(
        import_saved_history(&mut store, &port).expect_err("an unknown field is refused"),
        SemanticHistoryError::Corrupt
    );
    let mut doc = document(&saved);
    doc["events"][0]["input"]["invented"] = serde_json::json!(true);
    let port = Saved::new(fixture::binding())
        .holding_bytes("batch_0001", serde_json::to_vec(&doc).expect("bytes"));
    assert_eq!(
        import_saved_history(&mut store, &port).expect_err("an unknown field is refused"),
        SemanticHistoryError::Corrupt
    );
    let mut doc = document(&saved);
    doc["events"][0]["causal_parent"]["invented"] = serde_json::json!(true);
    let port = Saved::new(fixture::binding())
        .holding_bytes("batch_0001", serde_json::to_vec(&doc).expect("bytes"));
    assert_eq!(
        import_saved_history(&mut store, &port).expect_err("an unknown field is refused"),
        SemanticHistoryError::Corrupt
    );
}

#[test]
fn an_import_batch_that_is_not_a_document_is_refused() {
    let mut store = fixture::store();
    // An empty or oversized document is refused on its size alone, before it is ever parsed.
    for bytes in [Vec::new(), vec![b'{'; MAX_HISTORY_IMPORT_BYTES + 1]] {
        let port = Saved::new(fixture::binding()).holding_bytes("batch_0001", bytes);
        assert_eq!(
            import_saved_history(&mut store, &port).expect_err("the bytes are refused"),
            SemanticHistoryError::Bounds
        );
    }
    let port = Saved::new(fixture::binding()).holding_bytes("batch_0001", b"not json".to_vec());
    assert_eq!(
        import_saved_history(&mut store, &port).expect_err("the bytes are refused"),
        SemanticHistoryError::Corrupt
    );
}

#[test]
fn an_import_that_names_no_batches_or_too_many_is_refused() {
    let mut store = fixture::store();
    let empty = Saved::new(fixture::binding());
    assert_eq!(
        import_saved_history(&mut store, &empty).expect_err("an empty import is refused"),
        SemanticHistoryError::Bounds
    );
    let saved = batch("capture_0001", fixture::ROOT, 1);
    let mut crowded = Saved::new(fixture::binding());
    for index in 0..=20_u64 {
        crowded = crowded.holding(&format!("batch_{index:04}"), &saved);
    }
    assert_eq!(
        import_saved_history(&mut store, &crowded).expect_err("too many batches are refused"),
        SemanticHistoryError::Bounds
    );
}

#[test]
fn a_batch_carrying_more_events_than_the_bound_is_refused() {
    let mut store = fixture::store();
    let mut saved = batch("capture_0001", fixture::ROOT, 0);
    // One event past the bound, in a document that is still inside the byte bound, so only the
    // event count can refuse it.
    saved.events = (1..=257_u64)
        .map(|sequence| {
            imported(
                &format!("event_{sequence}"),
                SemanticHistoryKind::CardPlayed,
                sequence,
                None,
            )
        })
        .collect();
    let bytes = saved.encode().expect("encode");
    assert!(bytes.len() < MAX_HISTORY_IMPORT_BYTES);
    assert_eq!(refused(&mut store, &saved), SemanticHistoryError::Bounds);
    assert_eq!(store.len(fixture::ROOT).expect("len"), 0);
}

#[test]
fn a_batch_identity_or_capture_identity_that_is_not_opaque_is_refused() {
    let mut store = fixture::store();
    let saved = batch("capture_0001", fixture::ROOT, 1);
    let port = Saved::new(fixture::binding()).holding("../capture", &saved);
    assert_eq!(
        import_saved_history(&mut store, &port).expect_err("a path is refused"),
        SemanticHistoryError::NonOpaqueIdentity("import.batch_id")
    );
    let mut doc = document(&saved);
    doc["capture_id"] = serde_json::json!("/saves/run.json");
    let port = Saved::new(fixture::binding())
        .holding_bytes("batch_0001", serde_json::to_vec(&doc).expect("bytes"));
    assert_eq!(
        import_saved_history(&mut store, &port).expect_err("a path is refused"),
        SemanticHistoryError::NonOpaqueIdentity("import.capture_id")
    );
}
