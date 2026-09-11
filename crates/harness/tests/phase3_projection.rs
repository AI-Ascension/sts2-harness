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

fn source(id: &str, generation: u64) -> MemoryEntry {
    MemoryEntry::new(
        scope(),
        id,
        format!("record-{id}"),
        MemoryKind::HistoricalObservation,
        EvidenceStatus::Observed,
        "branch-a",
        id,
        format!("content-{id}").into_bytes(),
        generation,
        generation,
        generation,
        NOW,
        LATER,
        "synthetic-v1",
        false,
    )
}

#[test]
fn interrupted_rebuild_keeps_old_projection_and_authority() {
    let mut corpus = MemoryCorpus::with_limits(scope(), 8, 4096).expect("corpus");
    corpus.admit(source("root", 1)).expect("root");
    let before = corpus.backup().expect("before");
    let mut store = ProjectionStore::default();
    store.set_failpoint(Some(ProjectionFailpoint::BeforeSwap));
    assert_eq!(store.rebuild(&corpus), Err(MemoryError::PublicationFailed));
    assert!(store.current().is_none());
    assert_eq!(corpus.backup().expect("after"), before);
    let first = store.rebuild(&corpus).expect("first projection");
    corpus.admit(source("next", 2)).expect("next");
    store.set_failpoint(Some(ProjectionFailpoint::BeforeSwap));
    assert_eq!(store.rebuild(&corpus), Err(MemoryError::PublicationFailed));
    assert_eq!(store.current(), Some(&first));
    assert_eq!(
        store.read_refs(&corpus),
        Err(MemoryError::ProjectionUnavailable)
    );
}

#[test]
fn corrupt_projection_fails_closed_without_returning_references() {
    let mut corpus = MemoryCorpus::with_limits(scope(), 8, 4096).expect("corpus");
    corpus.admit(source("root", 1)).expect("root");
    let mut store = ProjectionStore::default();
    store.rebuild(&corpus).expect("projection");
    store.corrupt_for_test();
    assert_eq!(
        store.read_refs(&corpus),
        Err(MemoryError::ProjectionUnavailable)
    );
}

#[test]
fn outbox_replay_filters_future_unknown_and_revoked_references() {
    let mut corpus = MemoryCorpus::with_limits(scope(), 8, 4096).expect("corpus");
    let root = source("root", 1);
    let root_ref = root.reference();
    let next = source("next", 1);
    let next_ref = next.reference();
    corpus.admit(root).expect("root");
    corpus.admit(next).expect("next");
    let mut store = ProjectionStore::default();
    store.rebuild(&corpus).expect("projection");
    corpus
        .revoke(std::slice::from_ref(&root_ref), NOW)
        .expect("revoke");
    store.enqueue(root_ref).expect("revoked intent");
    store.enqueue(next_ref.clone()).expect("valid intent");
    store
        .enqueue(MemoryRef::new("unknown", 1, sha256_hex("unknown")))
        .expect("unknown intent");
    store
        .enqueue(MemoryRef::new("next", 2, sha256_hex("future")))
        .expect("future intent");
    let before = corpus.backup().expect("before replay");
    store.replay_outbox(&corpus).expect("replay");
    assert_eq!(store.read_refs(&corpus), Ok(vec![next_ref]));
    assert_eq!(corpus.backup().expect("after replay"), before);
}
