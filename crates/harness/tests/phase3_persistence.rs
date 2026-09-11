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

fn source(id: &str, text: &str, seq: u64) -> MemoryEntry {
    MemoryEntry::new(
        scope(),
        id,
        format!("record-{id}"),
        MemoryKind::HistoricalObservation,
        EvidenceStatus::Observed,
        "branch-a",
        id,
        text.as_bytes().to_vec(),
        seq,
        seq,
        1,
        NOW,
        LATER,
        "synthetic-v1",
        false,
    )
}

#[test]
fn durable_reload_applies_revocations_before_admitting_dependents() {
    let mut store = DurableMemoryStore::open(":memory:", scope(), [0x57; 32]).expect("store");
    let root = source("root", "root bytes", 1);
    let root_ref = root.reference();
    store.publish(root).expect("root");
    let mut child = source("child", "derived bytes", 2);
    child.kind = MemoryKind::Extract;
    child.evidence = EvidenceStatus::Derived;
    child.parents = vec![MemoryParent::from(&root_ref)];
    child.lineage_depth = 1;
    store.publish(child).expect("child");
    store
        .revoke(std::slice::from_ref(&root_ref), 1)
        .expect("revoke");

    let loaded = store.load_corpus().expect("reload");
    assert_eq!(loaded.entries().count(), 0);
    assert_eq!(
        loaded.read_content(&root_ref, "branch-a", 10, 1, NOW),
        Err(MemoryError::Revoked)
    );
}
