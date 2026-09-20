// SPDX-License-Identifier: MIT

//! Lineage a restored history must refuse.
//!
//! An edge and a branch must agree in both directions, exactly one branch is the run's root, and no
//! ancestry is deeper than the bound. A document that disagrees would leave two histories that
//! cannot be ordered against each other, so it is refused rather than repaired.

#![allow(clippy::expect_used, dead_code)]

use sts2_harness::semantic_history::{SemanticHistoryError, SemanticHistoryKind};

#[path = "support/semantic_history_fixture.rs"]
mod fixture;

#[test]
fn a_lineage_deeper_than_the_bound_is_refused_on_restore() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    let mut doc = fixture::document(&store);
    // A chain of empty branches exceeds the depth bound without touching any record, so only the
    // lineage itself can refuse it.
    let mut lineage = vec![serde_json::json!({
        "branch_id": fixture::ROOT,
        "parent_branch_id": serde_json::Value::Null,
        "fork_sequence": 0,
        "authority_epoch": 1,
    })];
    let mut branches = vec![doc["branches"][0].clone()];
    let mut parent = fixture::ROOT.to_owned();
    for depth in 1..=20_u64 {
        let child = format!("branch_{depth}");
        lineage.push(serde_json::json!({
            "branch_id": child,
            "parent_branch_id": parent,
            "fork_sequence": depth,
            "authority_epoch": 1,
        }));
        branches.push(serde_json::json!({"branch_id": child, "events": []}));
        parent = child;
    }
    doc["lineage"] = serde_json::Value::Array(lineage);
    doc["branches"] = serde_json::Value::Array(branches);
    assert_eq!(fixture::refused(&doc), SemanticHistoryError::Lineage);
}

#[test]
fn a_branch_that_lost_its_edge_is_refused_on_restore() {
    let mut store = fixture::store();
    store.fork("branch_child", fixture::ROOT, 5).expect("fork");
    let mut doc = fixture::document(&store);
    doc["lineage"] = serde_json::json!([doc["lineage"][1].clone()]);
    assert_eq!(fixture::refused(&doc), SemanticHistoryError::Lineage);
}

#[test]
fn a_branch_edge_that_names_no_branch_is_refused_on_restore() {
    let mut store = fixture::store();
    store.fork("branch_child", fixture::ROOT, 5).expect("fork");
    let mut doc = fixture::document(&store);
    // The edge still counts, so only the branch it names can reveal that it reaches nothing.
    doc["lineage"][1]["branch_id"] = serde_json::json!("branch_unknown");
    assert_eq!(fixture::refused(&doc), SemanticHistoryError::Lineage);
}

#[test]
fn a_second_root_branch_is_refused_on_restore() {
    let mut store = fixture::store();
    store.fork("branch_child", fixture::ROOT, 5).expect("fork");
    let mut doc = fixture::document(&store);
    doc["lineage"][1]["parent_branch_id"] = serde_json::Value::Null;
    assert_eq!(fixture::refused(&doc), SemanticHistoryError::Lineage);
}

#[test]
fn a_lineage_edge_from_an_epoch_the_store_never_reached_is_refused_on_restore() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    let mut doc = fixture::document(&store);
    doc["lineage"][0]["authority_epoch"] = serde_json::json!(2);
    assert_eq!(fixture::refused(&doc), SemanticHistoryError::Epoch);
}

#[test]
fn a_lineage_edge_that_is_not_an_opaque_identity_is_refused_on_restore() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    let mut doc = fixture::document(&store);
    // The branch and its record are renamed together, so only the edge's own identity can refuse it.
    doc["lineage"][0]["branch_id"] = serde_json::json!("/etc/passwd");
    doc["branches"][0]["branch_id"] = serde_json::json!("/etc/passwd");
    doc["branches"][0]["events"][0]["branch_id"] = serde_json::json!("/etc/passwd");
    assert_eq!(
        fixture::refused(&doc),
        SemanticHistoryError::NonOpaqueIdentity("lineage.branch_id")
    );
}

#[test]
fn a_cyclic_lineage_is_refused_on_restore() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    let mut doc = fixture::document(&store);
    // Two branches that each forked from the other: one root is still present, so only the ancestry
    // walk can reveal that neither of them ends.
    doc["lineage"] = serde_json::json!([
        {
            "branch_id": fixture::ROOT,
            "parent_branch_id": serde_json::Value::Null,
            "fork_sequence": 0,
            "authority_epoch": 1
        },
        {
            "branch_id": "branch_a",
            "parent_branch_id": "branch_b",
            "fork_sequence": 1,
            "authority_epoch": 1
        },
        {
            "branch_id": "branch_b",
            "parent_branch_id": "branch_a",
            "fork_sequence": 1,
            "authority_epoch": 1
        }
    ]);
    doc["branches"] = serde_json::json!([
        doc["branches"][0].clone(),
        {"branch_id": "branch_a", "events": []},
        {"branch_id": "branch_b", "events": []}
    ]);
    assert_eq!(fixture::refused(&doc), SemanticHistoryError::Lineage);
}
