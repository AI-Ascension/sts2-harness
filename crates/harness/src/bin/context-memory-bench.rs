// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use serde_json::json;
use std::time::{Duration, Instant};
use sts2_harness::context_memory::*;

const NOW: &str = "2026-09-10T12:00:00Z";
const LATER: &str = "2026-09-11T12:00:00Z";
const SAMPLES: usize = 32;

fn main() {
    let scope = MemoryScope::new(
        "project-fixture",
        "run-fixture",
        "episode-fixture",
        "agent-fixture",
    );
    let mut corpus = MemoryCorpus::with_limits(scope.clone(), 256, 64 * 1024).expect("corpus");
    for index in 0..64_u64 {
        let entry = MemoryEntry::new(
            scope.clone(),
            format!("source-{index}"),
            format!("record-{index}"),
            MemoryKind::HistoricalObservation,
            EvidenceStatus::Observed,
            "branch-a",
            format!("source-{index}"),
            format!("settled historical action {index}").into_bytes(),
            index + 1,
            index + 1,
            1,
            NOW,
            LATER,
            "synthetic-v1",
            false,
        );
        corpus.admit(entry).expect("admit");
    }
    let query = MemoryQuery {
        schema: MEMORY_QUERY_SCHEMA.to_owned(),
        query_id: "bench-query".to_owned(),
        scope,
        branch_id: "branch-a".to_owned(),
        query: "settled action".to_owned(),
        cutoff: 64,
        corpus_generation: 1,
        ranker_version: "lexical-v1".to_owned(),
        limit: 8,
        max_candidates: 64,
        effect_class: "local_read_no_inference".to_owned(),
    };
    let baseline = median((0..SAMPLES).map(|_| {
        let start = Instant::now();
        let _ = serde_json::to_vec(&query).expect("query");
        start.elapsed()
    }));
    let retrieval = median((0..SAMPLES).map(|_| {
        let start = Instant::now();
        let _ = corpus.retrieve(&query, NOW).expect("retrieve");
        start.elapsed()
    }));
    println!(
        "{}",
        json!({
            "schema": "ascension.context-memory.measurement.v1",
            "fixture": "synthetic-memory-corpus",
            "samples": SAMPLES,
            "baseline_p50_us": baseline.as_micros(),
            "memory_retrieval_p50_us": retrieval.as_micros(),
            "corpus_entries": corpus.entries().count(),
            "corpus_bytes": corpus.total_bytes(),
            "summary_maintenance_bytes": 0,
            "summary_calls": 0,
            "cache_hits": 0,
            "provider_class": "none_local_lexical_only"
        })
    );
}

fn median<I>(durations: I) -> Duration
where
    I: Iterator<Item = Duration>,
{
    let mut values = durations.collect::<Vec<_>>();
    values.sort_unstable();
    values[values.len() / 2]
}
