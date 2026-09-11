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

fn source(id: &str, observed: u64) -> MemoryEntry {
    MemoryEntry::new(
        scope(),
        id,
        format!("record-{id}"),
        MemoryKind::HistoricalObservation,
        EvidenceStatus::Observed,
        "branch-a",
        id,
        b"settled action".to_vec(),
        observed,
        observed,
        observed,
        NOW,
        LATER,
        "synthetic-v1",
        false,
    )
}

fn query(corpus: &MemoryCorpus, scope: MemoryScope, cutoff: u64) -> MemoryQuery {
    MemoryQuery {
        schema: MEMORY_QUERY_SCHEMA.to_owned(),
        query_id: "mutation-query".to_owned(),
        scope,
        branch_id: "branch-a".to_owned(),
        query: "settled".to_owned(),
        cutoff,
        corpus_generation: corpus.generation(),
        ranker_version: "lexical-v1".to_owned(),
        limit: 8,
        max_candidates: 64,
        effect_class: "local_read_no_inference".to_owned(),
    }
}

#[test]
fn causal_scope_and_revocation_mutations_are_detected() {
    let mut corpus = MemoryCorpus::with_limits(scope(), 8, 4096).expect("corpus");
    let root = source("root", 1);
    let root_ref = root.reference();
    corpus.admit(root).expect("root");
    corpus.admit(source("future", 9)).expect("future");
    let response = corpus
        .retrieve(&query(&corpus, scope(), 1), NOW)
        .expect("query");
    assert_eq!(response.results.len(), 1);
    assert_eq!(
        corpus.retrieve(
            &query(
                &corpus,
                MemoryScope::new("other", "run", "episode", "agent"),
                1
            ),
            NOW
        ),
        Err(MemoryError::PermissionDenied)
    );
    corpus
        .revoke(std::slice::from_ref(&root_ref), NOW)
        .expect("revoke");
    assert!(
        corpus
            .retrieve(&query(&corpus, scope(), 1), NOW)
            .expect("revoked query")
            .results
            .is_empty()
    );
}

#[test]
fn binding_and_exact_resume_mutations_are_detected() {
    let mut bindings = AtomicBindingStore::default();
    let binding = MemoryBinding {
        schema: MEMORY_BINDING_SCHEMA.to_owned(),
        binding_id: "binding-1".to_owned(),
        phase2_revision_id: "revision-1".to_owned(),
        phase2_preview_id: "preview-1".to_owned(),
        policy_id: "policy-1".to_owned(),
        policy_version: 1,
        selection_sha256: sha256_hex("selection"),
        audit_sha256: sha256_hex("audit"),
    };
    bindings.set_failpoint(Some(BindingFailpoint::BeforeCommit));
    assert!(matches!(
        bindings.commit(binding.clone()),
        Err(MemoryError::PublicationFailed)
    ));
    assert!(bindings.binding("binding-1").is_none());
    assert!(bindings.commit(binding).is_ok());
}

#[test]
fn map_bundle_swap_is_atomic_and_generation_bound() {
    let bundle = MapBundle {
        schema: "original.synthetic-map.v1".to_owned(),
        bundle_id: "map-1".to_owned(),
        generation: 1,
        scope: scope(),
        nodes: vec![MapNode {
            id: "room-1".to_owned(),
            visible: true,
            kind: "room".to_owned(),
            outcome: Some("unknown".to_owned()),
        }],
        edges: Vec::new(),
        legal_next_nodes: vec!["room-1".to_owned()],
        provenance: "synthetic-visible-map".to_owned(),
    };
    let mut store = AtomicMapBundleStore::default();
    store.set_failpoint(Some(MapBundleFailpoint::BeforeSwap));
    assert_eq!(
        store.commit(bundle.clone(), &scope(), 1),
        Err(MemoryError::PublicationFailed)
    );
    assert!(store.current().is_none());
    store.commit(bundle, &scope(), 1).expect("swap");
    assert_eq!(
        store.current().map(|item| item.bundle_id.as_str()),
        Some("map-1")
    );
}
