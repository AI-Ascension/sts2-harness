// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use sts2_harness::{
    ExactCheckpointId, ExactStateDigest, LineageError, OccurrenceGraph, OccurrenceId,
    OccurrenceRecord,
};

fn digest(seed: char) -> ExactStateDigest {
    ExactStateDigest::parse(&format!(
        "asc-state:v1:sha256:{}",
        seed.to_string().repeat(64)
    ))
    .expect("digest is valid")
}

fn id(value: &str) -> OccurrenceId {
    OccurrenceId::parse(value).expect("identifier is valid")
}

fn record(
    occurrence: &str,
    parent: Option<&str>,
    action: Option<&str>,
    state: char,
) -> OccurrenceRecord {
    OccurrenceRecord {
        occurrence_id: id(occurrence),
        parent: parent.map(id),
        parent_checkpoint: None,
        action_key: action.map(str::to_owned),
        state_digest: digest(state),
        experiment_id: Some("trial-a".to_owned()),
    }
}

fn graph(records: Vec<OccurrenceRecord>) -> OccurrenceGraph {
    let mut graph = OccurrenceGraph::new();
    for record in records {
        graph.insert(record).expect("record inserts");
    }
    graph
}

#[test]
fn shared_state_through_distinct_parents_stays_distinct() {
    let graph = graph(vec![
        record("run:root", None, None, 'a'),
        record("run:left", Some("run:root"), Some("play_card"), 'c'),
        record("run:right", Some("run:root"), Some("end_turn"), 'c'),
    ]);
    assert_eq!(graph.len(), 3);
    let shared = graph.occurrences_with_state(&digest('c'));
    assert_eq!(shared, vec![id("run:left"), id("run:right")]);
    assert_eq!(
        graph.ancestors(&id("run:left")).expect("ancestry"),
        vec![id("run:root"), id("run:left")]
    );
    assert_eq!(
        graph.ancestors(&id("run:right")).expect("ancestry"),
        vec![id("run:root"), id("run:right")]
    );
    assert_eq!(graph.root(&id("run:right")).expect("root"), id("run:root"));
}

#[test]
fn parents_must_be_inserted_before_children() {
    let mut graph = OccurrenceGraph::new();
    let error = graph
        .insert(record(
            "run:child",
            Some("run:missing"),
            Some("play_card"),
            'b',
        ))
        .expect_err("unknown parent rejected");
    assert_eq!(error, LineageError::UnknownParent);
    assert!(graph.is_empty());
}

#[test]
fn duplicate_occurrences_and_unknown_lookups_are_rejected() {
    let mut graph = graph(vec![record("run:root", None, None, 'a')]);
    assert_eq!(
        graph
            .insert(record("run:root", None, None, 'd'))
            .expect_err("duplicate rejected"),
        LineageError::DuplicateOccurrence
    );
    assert_eq!(
        graph.ancestors(&id("run:missing")).expect_err("unknown"),
        LineageError::UnknownOccurrence
    );
}

#[test]
fn malformed_lineage_records_are_rejected() {
    let mut graph = OccurrenceGraph::new();
    assert_eq!(
        graph
            .insert(record("run:bad", None, Some("play_card"), 'a'))
            .expect_err("root with action rejected"),
        LineageError::InvalidRecord
    );
    let mut with_checkpoint = record("run:root", None, None, 'a');
    with_checkpoint.parent_checkpoint = Some(
        ExactCheckpointId::parse(&format!("asc-checkpoint:v1:sha256:{}", "a".repeat(64)))
            .expect("checkpoint id"),
    );
    assert_eq!(
        graph
            .insert(with_checkpoint)
            .expect_err("root checkpoint rejected"),
        LineageError::InvalidRecord
    );
}

#[test]
fn invalid_identifiers_are_rejected() {
    for candidate in ["", " ", "run/with space", "run/slash"] {
        assert_eq!(
            OccurrenceId::parse(candidate).expect_err("identifier rejected"),
            LineageError::InvalidIdentifier
        );
    }
    let long = "a".repeat(300);
    assert_eq!(
        OccurrenceId::parse(&long).expect_err("oversized identifier rejected"),
        LineageError::InvalidIdentifier
    );
}

#[test]
fn ancestry_grouping_keeps_duplicate_content_in_one_split() {
    let graph = graph(vec![
        record("exp-a:root", None, None, 'a'),
        record("exp-a:one", Some("exp-a:root"), Some("play_card"), 'c'),
        record("exp-a:two", Some("exp-a:one"), Some("end_turn"), 'c'),
        record("exp-b:root", None, None, 'a'),
        record("exp-b:one", Some("exp-b:root"), Some("play_card"), 'd'),
    ]);
    let groups = graph.group_by_root().expect("groups build");
    assert_eq!(groups.len(), 2);
    let first = groups.get(&id("exp-a:root")).expect("group a");
    assert_eq!(first.len(), 3);
    assert!(first.contains(&id("exp-a:two")));
    let second = groups.get(&id("exp-b:root")).expect("group b");
    assert_eq!(second.len(), 2);
    assert!(!second.contains(&id("exp-a:two")));
    assert_eq!(graph.occurrences_with_state(&digest('a')).len(), 2);
}
