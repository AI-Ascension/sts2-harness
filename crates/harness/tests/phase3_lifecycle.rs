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

fn source(id: &str, text: &str, generation: u64) -> MemoryEntry {
    MemoryEntry::new(
        scope(),
        id,
        format!("record-{id}"),
        MemoryKind::HistoricalObservation,
        EvidenceStatus::Observed,
        "branch-a",
        id,
        text.as_bytes().to_vec(),
        1,
        1,
        generation,
        NOW,
        LATER,
        "synthetic-v1",
        false,
    )
}

fn query(corpus: &MemoryCorpus, text: &str) -> MemoryQuery {
    MemoryQuery {
        schema: MEMORY_QUERY_SCHEMA.to_owned(),
        query_id: format!("query-{}", sha256_hex(text)[..8].to_owned()),
        scope: scope(),
        branch_id: "branch-a".to_owned(),
        query: text.to_owned(),
        cutoff: 10,
        corpus_generation: corpus.generation(),
        ranker_version: "lexical-v1".to_owned(),
        limit: 8,
        max_candidates: 64,
        effect_class: "local_read_no_inference".to_owned(),
    }
}

#[test]
fn restore_unions_revocation_ledger_before_readers() {
    let mut corpus = MemoryCorpus::with_limits(scope(), 8, 4096).expect("corpus");
    let root = source("root", "HP loss was 2", 1);
    let root_ref = root.reference();
    corpus.admit(root).expect("admit");
    let backup = corpus.backup().expect("backup");
    corpus
        .revoke(std::slice::from_ref(&root_ref), NOW)
        .expect("revoke");
    corpus.restore(&backup).expect("restore");
    assert_eq!(
        corpus.read_content(&root_ref, "branch-a", 10, 1, NOW),
        Err(MemoryError::Revoked)
    );
    assert_eq!(corpus.cleanup_revoked(), 0);
    let tombstone_backup = corpus.backup().expect("tombstone backup");
    let mut restored = MemoryCorpus::with_limits(scope(), 8, 4096).expect("restored corpus");
    restored
        .restore(&tombstone_backup)
        .expect("restore tombstone");
    assert_eq!(
        restored.read_content(&root_ref, "branch-a", 10, 1, NOW),
        Err(MemoryError::Revoked)
    );
}

#[test]
fn projection_cache_and_no_match_have_explicit_outcomes() {
    let mut corpus = MemoryCorpus::with_limits(scope(), 8, 4096).expect("corpus");
    corpus
        .admit(source("root", "settled action", 1))
        .expect("admit");
    let q = query(&corpus, "settled");
    let response = corpus.retrieve(&q, NOW).expect("retrieve");
    assert_eq!(retrieval_outcome(&response), RetrievalOutcome::Complete);
    assert!(
        response.results[0]
            .reasons
            .contains(&"relevance_signal_not_confidence".to_owned())
    );
    let no_match = corpus
        .retrieve(&query(&corpus, "nonexistent"), NOW)
        .expect("retrieve");
    assert_eq!(retrieval_outcome(&no_match), RetrievalOutcome::NoMatch);
    let mut cache = RetrievalCache::new(2).expect("cache");
    cache.put(&q, response);
    assert!(matches!(cache.get(&q, &corpus), CacheLookup::Hit(_)));
    corpus.set_projection_health(false);
    assert_eq!(cache.get(&q, &corpus), CacheLookup::Invalidated);
    corpus.set_projection_health(true);
    let projection = corpus.rebuild_projection().expect("projection");
    assert_eq!(projection.reference_count, 1);
    assert_eq!(projection.corpus_generation, corpus.generation());
}

#[test]
fn bounded_scan_reports_timeout_and_rejects_unknown_rankers() {
    let mut corpus = MemoryCorpus::with_limits(scope(), 8, 4096).expect("corpus");
    corpus
        .admit(source("a", "settled action", 1))
        .expect("admit");
    corpus.admit(source("b", "other action", 2)).expect("admit");
    let q = query(&corpus, "settled");
    let (_, outcome) = corpus
        .retrieve_with_scan_budget(&q, NOW, 1)
        .expect("bounded retrieve");
    assert_eq!(outcome, RetrievalOutcome::TimeLimited);
    let mut unknown = q;
    unknown.ranker_version = "future-ranker".to_owned();
    assert_eq!(
        corpus.retrieve(&unknown, NOW),
        Err(MemoryError::InvalidQuery)
    );
}

#[test]
fn unknown_summary_reservation_remains_in_global_budget() {
    let mut ledger = MemoryBudgetLedger::new(1, 128).expect("ledger");
    ledger.reserve("job-1", 32, 32).expect("reserve");
    ledger.mark_unknown("job-1").expect("unknown");
    assert_eq!(ledger.active_jobs(), 1);
    assert_eq!(
        ledger.reserve("job-2", 1, 1),
        Err(MemoryError::BudgetExceeded)
    );
    ledger
        .finish("job-1", ReservationState::Released)
        .expect("release");
    assert_eq!(ledger.active_jobs(), 0);
}

#[test]
fn binding_transaction_is_all_or_nothing_and_idempotent() {
    let mut store = AtomicBindingStore::default();
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
    store.set_failpoint(Some(BindingFailpoint::BeforeCommit));
    assert_eq!(
        store.commit(binding.clone()),
        Err(MemoryError::PublicationFailed)
    );
    assert!(store.binding("binding-1").is_none());
    assert_eq!(store.commit(binding.clone()), Ok(binding.clone()));
    assert_eq!(store.commit(binding.clone()), Ok(binding));
    let mut downgrade = store.binding("binding-1").expect("binding").clone();
    downgrade.audit_sha256 = sha256_hex("legacy-client");
    assert_eq!(store.commit(downgrade), Err(MemoryError::Conflict));
}

#[test]
fn critical_facts_and_review_versions_fence_poisoned_edits() {
    let mut corpus = MemoryCorpus::with_limits(scope(), 8, 4096).expect("corpus");
    let item = source("root", "HP loss was 2, and the action settled.", 1);
    let reference = item.reference();
    corpus.admit(item).expect("admit");
    let proposal = corpus
        .exact_extract(
            "proposal-1",
            std::slice::from_ref(&reference),
            "branch-a",
            10,
            1,
            NOW,
            NOW,
            LATER,
        )
        .expect("extract");
    verify_critical_facts(
        &proposal,
        &corpus,
        &[CriticalFact {
            source: reference.clone(),
            literal: "HP loss was 2".to_owned(),
        }],
    )
    .expect("facts");
    let mut ledger = ImmutableReviewLedger::default();
    let review = MemoryReview {
        schema: MEMORY_REVIEW_SCHEMA.to_owned(),
        review_id: "review-1".to_owned(),
        proposal_id: proposal.proposal_id.clone(),
        proposal_version: proposal.version,
        proposal_sha256: proposal.sha256.clone(),
        scope: scope(),
        source_manifest_sha256: source_manifest_digest(std::slice::from_ref(&reference)),
        revocation_epoch: corpus.revocation_epoch(),
        reviewer_ref: "reviewer-1".to_owned(),
        decision: ReviewDecision::Admit,
        support_check: SupportCheck::ExactExtractChecked,
        reason_codes: vec!["critical_facts_preserved".to_owned()],
        created_at: NOW.to_owned(),
        creates_active_revision: false,
    };
    ledger.record(&proposal, review, &corpus).expect("review");
    let revised = ledger
        .revise(&proposal, b"HP loss was 20".to_vec(), &corpus, NOW)
        .expect("revise");
    assert_eq!(revised.version, 2);
    assert!(
        ledger
            .review(&proposal.proposal_id, revised.version)
            .is_none()
    );
    assert_eq!(
        verify_critical_facts(
            &revised,
            &corpus,
            &[CriticalFact {
                source: reference,
                literal: "HP loss was 2".to_owned()
            }],
        ),
        Err(MemoryError::InvalidProposal)
    );
}

#[test]
fn policy_realization_and_map_attachment_keep_generation_pins() {
    let mut corpus = MemoryCorpus::with_limits(scope(), 8, 4096).expect("corpus");
    corpus
        .admit(source("root", "settled action", 1))
        .expect("admit");
    let policy = MemoryPolicy {
        schema: MEMORY_POLICY_SCHEMA.to_owned(),
        policy_id: "policy-1".to_owned(),
        version: 1,
        scope: scope(),
        mode: PolicyMode::ManualSnapshot,
        status: PolicyStatus::Approved,
        phase2_revision_id: Some("revision-1".to_owned()),
        corpus_generation: corpus.generation(),
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
    let realized = realize_policy(
        &corpus,
        &policy,
        "selection-1",
        &query(&corpus, "settled"),
        b"mandatory".to_vec(),
        sha256_hex("phase2"),
        "prepared-1",
        LATER,
        NOW,
    )
    .expect("realize");
    assert_eq!(realized.selection.selected_sources.len(), 1);
    assert_eq!(realized.selection.effect_class, "local_preparation_only");

    let graph = MapAttachment::new(
        MapAttachmentKind::Graph,
        "graph-1",
        1,
        b"graph".to_vec(),
        None,
    );
    let analysis = MapAttachment::new(
        MapAttachmentKind::Analysis,
        "analysis-1",
        1,
        b"analysis".to_vec(),
        None,
    );
    let graph_only = MapArtifactBundle {
        schema: MEMORY_MAP_BUNDLE_SCHEMA.to_owned(),
        bundle_id: "bundle-1".to_owned(),
        scope: scope(),
        generation: 1,
        graph,
        analysis,
        image: None,
        image_requested: false,
        graph_only_reason: Some("image capability unavailable".to_owned()),
    };
    assert!(graph_only.validate(&scope(), 1).is_ok());
    let mut requested = graph_only.clone();
    requested.image_requested = true;
    assert_eq!(
        requested.validate(&scope(), 1),
        Err(MemoryError::Unsupported)
    );
}
