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

#[test]
fn first_resume_consumes_exact_prepared_bytes_once() {
    let mut corpus = MemoryCorpus::with_limits(scope(), 8, 4096).expect("corpus");
    corpus
        .admit(MemoryEntry::new(
            scope(),
            "root",
            "record-root",
            MemoryKind::HistoricalObservation,
            EvidenceStatus::Observed,
            "branch-a",
            "root",
            b"settled action".to_vec(),
            1,
            1,
            1,
            NOW,
            LATER,
            "synthetic-v1",
            false,
        ))
        .expect("source");
    let policy = MemoryPolicy {
        schema: MEMORY_POLICY_SCHEMA.to_owned(),
        policy_id: "policy-1".to_owned(),
        version: 1,
        scope: scope(),
        mode: PolicyMode::ManualSnapshot,
        status: PolicyStatus::Approved,
        phase2_revision_id: Some("revision-1".to_owned()),
        corpus_generation: 1,
        rolling_same_episode_sources: false,
        cross_scope: false,
        approved_summary_catalog: Vec::new(),
        ranker_version: "lexical-v1".to_owned(),
        query_derivation_version: "derive-v1".to_owned(),
        max_candidates: 64,
        max_results: 8,
        max_selected: 8,
        optional_byte_budget: 128,
        fallback: SelectionFallback::Block,
        automatic_summary_activation: false,
        generate_during_selection: false,
        authorization_policy_version: "auth-v1".to_owned(),
    };
    let query = MemoryQuery {
        schema: MEMORY_QUERY_SCHEMA.to_owned(),
        query_id: "query-1".to_owned(),
        scope: scope(),
        branch_id: "branch-a".to_owned(),
        query: "settled".to_owned(),
        cutoff: 10,
        corpus_generation: 1,
        ranker_version: "lexical-v1".to_owned(),
        limit: 8,
        max_candidates: 64,
        effect_class: "local_read_no_inference".to_owned(),
    };
    let realized = realize_policy(
        &corpus,
        &policy,
        "selection-1",
        &query,
        b"mandatory".to_vec(),
        sha256_hex("phase2"),
        "prepared-1",
        LATER,
        NOW,
    )
    .expect("realize");
    let mut approvals = ApprovalStore::default();
    approvals
        .bind(
            "approval-1",
            &realized.selection,
            "preview-1",
            source_manifest_digest(&realized.selection.selected_sources),
        )
        .expect("bind");
    let held = approvals.commit_held("approval-1", 0, NOW).expect("hold");
    let mut ledger = FirstResumeLedger::default();
    ledger.prepare(&held, &realized.selection).expect("prepare");
    let (outcome, first) = ledger
        .submit_first(&held, 0, NOW, &realized.selection.rendered_bytes)
        .expect("submit");
    assert_eq!(outcome, ResumeOutcome::Submitted);
    assert_eq!(first.rendered_bytes, realized.selection.rendered_bytes);
    let (replay, duplicate) = ledger
        .submit_first(&held, 0, NOW, &realized.selection.rendered_bytes)
        .expect("replay");
    assert_eq!(replay, ResumeOutcome::AlreadySubmitted);
    assert!(duplicate.rendered_bytes.is_empty());
    assert!(ledger.is_consumed("approval-1"));
    assert_eq!(
        ledger.submit_first(&held, 1, NOW, &realized.selection.rendered_bytes),
        Err(MemoryError::StaleApproval)
    );
}
