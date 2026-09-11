// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::sync::{Arc, Barrier};
use std::thread;
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

fn source(id: &str, text: &str, observed: u64, generation: u64) -> MemoryEntry {
    MemoryEntry::new(
        scope(),
        id,
        format!("record-{id}"),
        MemoryKind::HistoricalObservation,
        EvidenceStatus::Observed,
        "branch-a",
        id,
        text.as_bytes().to_vec(),
        observed,
        observed,
        generation,
        NOW,
        LATER,
        "synthetic-v1",
        false,
    )
}

#[test]
fn revoke_and_dependent_publication_race_leaves_no_eligible_derivative() {
    let root = source("root", "settled action", 1, 1);
    let root_ref = root.reference();
    let mut initial = MemoryCorpus::with_limits(scope(), 8, 4096).expect("corpus");
    initial.admit(root).expect("root");
    let shared = ConcurrentMemoryCorpus::new(initial);
    let barrier = Arc::new(Barrier::new(3));
    let revoke_store = shared.clone();
    let revoke_barrier = barrier.clone();
    let revoke_root = root_ref.clone();
    let revoker = thread::spawn(move || {
        revoke_barrier.wait();
        revoke_store.revoke(std::slice::from_ref(&revoke_root), NOW)
    });
    let publish_store = shared.clone();
    let publish_barrier = barrier.clone();
    let publish_root = root_ref.clone();
    let publisher = thread::spawn(move || {
        let mut derived = source("derived", "settled action", 2, 2);
        derived.kind = MemoryKind::Extract;
        derived.evidence = EvidenceStatus::Derived;
        derived.parents = vec![MemoryParent::from(&publish_root)];
        derived.lineage_depth = 1;
        publish_barrier.wait();
        publish_store.admit(derived)
    });
    barrier.wait();
    let revoke_result = revoker.join().expect("revoker");
    let publish_result = publisher.join().expect("publisher");
    assert!(revoke_result.is_ok());
    assert!(matches!(publish_result, Ok(_) | Err(MemoryError::Revoked)));
    let query = MemoryQuery {
        schema: MEMORY_QUERY_SCHEMA.to_owned(),
        query_id: "race-query".to_owned(),
        scope: scope(),
        branch_id: "branch-a".to_owned(),
        query: "settled".to_owned(),
        cutoff: 10,
        corpus_generation: 2,
        ranker_version: "lexical-v1".to_owned(),
        limit: 8,
        max_candidates: 64,
        effect_class: "local_read_no_inference".to_owned(),
    };
    let response = shared.retrieve(&query, NOW).expect("retrieve");
    assert!(response.results.is_empty());
}
