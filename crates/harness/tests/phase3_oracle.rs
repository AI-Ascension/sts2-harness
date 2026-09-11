// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use sts2_harness::context_memory::*;

const NOW: &str = "2026-09-10T12:00:00Z";
const LATER: &str = "2026-09-11T12:00:00Z";

fn scope() -> MemoryScope {
    MemoryScope::new(
        "project-fixture",
        "run-fixture",
        "episode-fixture",
        "agent-fixture",
    )
}

fn entry(id: &str, branch: &str, observed: u64, generation: u64, protected: bool) -> MemoryEntry {
    MemoryEntry::new(
        scope(),
        id,
        format!("record-{id}"),
        MemoryKind::HistoricalObservation,
        EvidenceStatus::Observed,
        branch,
        id,
        b"settled HP action".to_vec(),
        observed,
        observed,
        generation,
        NOW,
        LATER,
        "synthetic-v1",
        protected,
    )
}

fn query(corpus: &MemoryCorpus, scope: MemoryScope) -> MemoryQuery {
    MemoryQuery {
        schema: MEMORY_QUERY_SCHEMA.to_owned(),
        query_id: "oracle-query".to_owned(),
        scope,
        branch_id: "branch-a".to_owned(),
        query: "settled action".to_owned(),
        cutoff: 10,
        corpus_generation: corpus.generation(),
        ranker_version: "lexical-v1".to_owned(),
        limit: 8,
        max_candidates: 64,
        effect_class: "local_read_no_inference".to_owned(),
    }
}

#[test]
fn independent_fixture_oracle_checks_membership_and_tie_order() {
    let mut corpus = MemoryCorpus::with_limits(scope(), 16, 4096).expect("corpus");
    for item in [
        entry("b", "branch-a", 4, 5, false),
        entry("a", "branch-a", 4, 5, false),
        entry("late", "branch-a", 3, 11, false),
        entry("future", "branch-a", 12, 12, false),
        entry("protected", "branch-a", 4, 5, true),
        entry("sibling", "branch-b", 4, 5, false),
    ] {
        corpus.admit(item).expect("admit fixture");
    }
    let mut pinned = query(&corpus, scope());
    pinned.corpus_generation = 10;
    let response = corpus.retrieve(&pinned, NOW).expect("retrieve");
    // These expected IDs are the hand-labelled oracle, independent of the ranker implementation.
    let expected = ["a", "b"];
    let actual = response
        .results
        .iter()
        .map(|result| result.source.entry_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
    assert_eq!(response.inference_calls, 0);
}

#[test]
fn held_out_labels_are_private_and_cutoff_scope_is_rechecked() {
    let evaluation_scope = MemoryScope::new(
        "project-fixture",
        "heldout-run",
        "episode-fixture",
        "evaluator",
    );
    let mut held_out = HeldOutEvaluation::new(evaluation_scope.clone(), 10).expect("lane");
    held_out
        .record_private_label("case-1", vec!["a".to_owned(), "b".to_owned()])
        .expect("label");
    let config = held_out
        .lane_config(EvaluationLane::Retrieval, sha256_hex("oracle"))
        .expect("config");
    assert!(
        serde_json::to_value(config)
            .expect("wire config")
            .get("labels")
            .is_none()
    );
    let corpus = MemoryCorpus::with_limits(scope(), 4, 1024).expect("corpus");
    let mut foreign_query = query(&corpus, evaluation_scope);
    foreign_query.corpus_generation = 1;
    assert_eq!(
        corpus.retrieve(&foreign_query, NOW),
        Err(MemoryError::PermissionDenied)
    );
}

#[test]
fn normalization_vectors_are_unicode_stable_and_bounded() {
    assert_eq!(
        normalize_terms("Äction, THE — settled").expect("terms"),
        vec!["äction", "settled"]
    );
    assert!(normalize_terms("the and of").expect("stopwords").is_empty());
    let oversized = "x ".repeat(MAX_CANDIDATES + 1);
    assert_eq!(normalize_terms(&oversized), Err(MemoryError::TooManyTerms));
}
