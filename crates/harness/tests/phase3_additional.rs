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

fn source(id: &str, text: &str) -> MemoryEntry {
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
        1,
        NOW,
        LATER,
        "synthetic-v1",
        false,
    )
}

fn job(id: &str, key: &str) -> SummaryJob {
    SummaryJob {
        schema: MEMORY_JOB_SCHEMA.to_owned(),
        job_id: id.to_owned(),
        scope: scope(),
        branch_id: "branch-a".to_owned(),
        sources: vec![MemoryRef::new("source", 1, sha256_hex("history"))],
        cutoff: 10,
        corpus_generation: 1,
        source_manifest_sha256: source_manifest_digest(&[MemoryRef::new(
            "source",
            1,
            sha256_hex("history"),
        )]),
        generator_profile: "fake-v1".to_owned(),
        generator_prompt_sha256: sha256_hex("prompt"),
        output_schema_sha256: sha256_hex("output"),
        idempotency_key: key.to_owned(),
        command_window_id: "window-1".to_owned(),
        state: JobState::Queued,
        attempt_id: None,
        provider_write_state: ProviderWriteState::NotStarted,
        input_bytes: 7,
        max_output_bytes: 8192,
        review_required: true,
        auto_apply: false,
        deadline_at: LATER.to_owned(),
        effect_class: "authorized_summary_generation_only".to_owned(),
    }
}

#[test]
fn summary_jobs_are_idempotent_and_expire_without_execution() {
    let mut jobs = SummaryJobStore::new(4).expect("jobs");
    let first = job("job-1", "key-1");
    assert_eq!(jobs.admit(first.clone()), Ok(true));
    assert_eq!(jobs.admit(first.clone()), Ok(false));
    let mut changed = first;
    changed.input_bytes = 6;
    assert_eq!(jobs.admit(changed), Err(MemoryError::JobConflict));

    let mut corpus = MemoryCorpus::with_limits(scope(), 4, 1024).expect("corpus");
    corpus.admit(source("source", "history")).expect("source");
    let mut peer = FakeSummaryPeer::new(b"summary".to_vec()).expect("peer");
    assert_eq!(
        jobs.execute_explicit("job-1", &corpus, &mut peer, LATER, true),
        Err(MemoryError::JobUnknown)
    );
    assert_eq!(peer.requests().len(), 0);
}

#[test]
fn identical_selection_inputs_have_identical_rendered_bytes_and_exclusions() {
    let mut corpus = MemoryCorpus::with_limits(scope(), 8, 4096).expect("corpus");
    corpus
        .admit(source("source", "settled action"))
        .expect("source");
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
        optional_byte_budget: 8,
        fallback: SelectionFallback::Block,
        automatic_summary_activation: false,
        generate_during_selection: false,
        authorization_policy_version: "auth-v1".to_owned(),
    };
    let query = MemoryQuery {
        schema: MEMORY_QUERY_SCHEMA.to_owned(),
        query_id: "selection-query".to_owned(),
        scope: scope(),
        branch_id: "branch-a".to_owned(),
        query: "settled".to_owned(),
        cutoff: 10,
        corpus_generation: corpus.generation(),
        ranker_version: "lexical-v1".to_owned(),
        limit: 8,
        max_candidates: 64,
        effect_class: "local_read_no_inference".to_owned(),
    };
    let first = realize_policy(
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
    .expect("first selection");
    let second = realize_policy(
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
    .expect("second selection");
    assert_eq!(
        first.selection.rendered_bytes,
        second.selection.rendered_bytes
    );
    assert_eq!(first.selection.exclusions, second.selection.exclusions);
    assert_eq!(
        first.selection.prepared_manifest_sha256,
        second.selection.prepared_manifest_sha256
    );
}

#[test]
fn image_requests_require_image_bytes_and_graph_only_discloses_the_limit() {
    let graph = MapAttachment::new(
        MapAttachmentKind::Graph,
        "graph-1",
        3,
        b"graph".to_vec(),
        None,
    );
    let analysis = MapAttachment::new(
        MapAttachmentKind::Analysis,
        "analysis-1",
        3,
        b"analysis".to_vec(),
        None,
    );
    let image_missing = MapArtifactBundle {
        schema: MEMORY_MAP_BUNDLE_SCHEMA.to_owned(),
        bundle_id: "bundle-1".to_owned(),
        scope: scope(),
        generation: 3,
        graph: graph.clone(),
        analysis: analysis.clone(),
        image: None,
        image_requested: true,
        graph_only_reason: None,
    };
    assert_eq!(
        image_missing.validate(&scope(), 3),
        Err(MemoryError::Unsupported)
    );
    let graph_only = MapArtifactBundle {
        image_requested: false,
        graph_only_reason: Some("image capability unavailable".to_owned()),
        ..image_missing
    };
    graph_only.validate(&scope(), 3).expect("graph-only bundle");
}
