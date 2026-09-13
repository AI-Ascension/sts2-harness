// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use sts2_harness::context_memory::*;

fn scope() -> MemoryScope {
    MemoryScope::new(
        "project-fixture",
        "run-fixture",
        "episode-fixture",
        "agent-fixture",
    )
}

fn source(id: &str, text: &str, observed: u64, generation: u64, protected: bool) -> MemoryEntry {
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
        observed.saturating_add(1),
        generation,
        "2026-09-10T10:00:00Z",
        "2026-09-11T12:00:00Z",
        "synthetic-v1",
        protected,
    )
}

#[test]
fn fixture_scope_filters_late_future_sibling_private_and_protected_sources() {
    let mut corpus = MemoryCorpus::with_limits(scope(), 16, 4096).expect("corpus");
    for (id, text, observed, generation, protected) in [
        ("hist", "HP loss was 2 and settled", 4, 5, false),
        ("late", "earlier event arrived late", 3, 11, false),
        ("future", "future reward is revealed", 12, 12, false),
        ("protected", "legal catalog remains complete", 10, 10, true),
    ] {
        corpus
            .admit(source(id, text, observed, generation, protected))
            .expect("admit");
    }
    let mut sibling = source("sibling", "different branch outcome", 4, 5, false);
    sibling.branch_id = "branch-b".to_owned();
    corpus.admit(sibling).expect("sibling");
    let mut private = source("private", "private reward", 4, 5, false);
    private.scope.agent_id = "other-agent".to_owned();
    assert_eq!(corpus.admit(private), Err(MemoryError::InvalidScope));

    let query = MemoryQuery {
        schema: MEMORY_QUERY_SCHEMA.to_owned(),
        query_id: "query-fixture".to_owned(),
        scope: scope(),
        branch_id: "branch-a".to_owned(),
        query: "reward settled".to_owned(),
        cutoff: 10,
        corpus_generation: 10,
        ranker_version: "lexical-v1".to_owned(),
        limit: 16,
        max_candidates: 64,
        effect_class: "local_read_no_inference".to_owned(),
    };
    let response = corpus
        .retrieve(&query, "2026-09-10T12:00:00Z")
        .expect("retrieve");
    assert_eq!(response.results.len(), 1);
    assert_eq!(response.results[0].source.entry_id, "hist");
    assert_eq!(response.inference_calls, 0);
}

#[test]
fn capabilities_publish_the_effective_policy_limits() {
    let corpus = MemoryCorpus::with_limits(scope(), 16, 4096).expect("corpus");
    let capabilities = corpus.capabilities();

    assert_eq!(
        capabilities.effective_limits.policy_schema,
        MEMORY_POLICY_SCHEMA
    );
    assert_eq!(capabilities.effective_limits.max_candidates, MAX_CANDIDATES);
    assert_eq!(capabilities.effective_limits.max_results, MAX_RESULTS);
    assert_eq!(capabilities.effective_limits.max_selected, MAX_SELECTED);
    assert_eq!(
        capabilities.effective_limits.optional_byte_budget,
        MAX_OPTIONAL_BYTES
    );
    assert_eq!(capabilities.effective_limits.max_entries_per_run, 16);
    assert_eq!(capabilities.effective_limits.max_corpus_bytes, 4096);
    capabilities.validate().expect("capability descriptor");
}

#[test]
fn schema_valid_policy_above_effective_limit_is_rejected_and_migrated_explicitly() {
    let corpus = MemoryCorpus::with_limits(scope(), 16, 4096).expect("corpus");
    let capabilities = corpus.capabilities();
    let policy = MemoryPolicy {
        schema: MEMORY_POLICY_SCHEMA.to_owned(),
        policy_id: "policy-migration".to_owned(),
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
        max_candidates: MAX_CANDIDATES,
        max_results: MAX_RESULTS,
        max_selected: MAX_SELECTED,
        optional_byte_budget: MEMORY_POLICY_SCHEMA_MAX_OPTIONAL_BYTES,
        fallback: SelectionFallback::Block,
        automatic_summary_activation: false,
        generate_during_selection: false,
        authorization_policy_version: "auth-v1".to_owned(),
    };
    policy.validate_schema().expect("portable schema");
    assert_eq!(
        policy.validate_against_capabilities(&corpus, &capabilities),
        Err(MemoryError::CapabilityLimitExceeded {
            limit: "optional_byte_budget".to_owned(),
            requested: MEMORY_POLICY_SCHEMA_MAX_OPTIONAL_BYTES,
            effective: MAX_OPTIONAL_BYTES,
        })
    );

    let original = serde_json::to_vec(&policy).expect("policy bytes");
    let mut proposal =
        PolicyMigrationProposal::new(&policy, &capabilities, "migration-1").expect("proposal");
    assert_eq!(proposal.original_policy_bytes(), original.as_slice());
    proposal.approve("approval-1").expect("approve");
    let mut target = policy.clone();
    target.version = 2;
    target.optional_byte_budget = MAX_OPTIONAL_BYTES;
    let adopted = proposal
        .adopt(&target, &capabilities, "approval-1")
        .expect("adopt");
    assert_eq!(adopted, target);
    assert_eq!(proposal.original_policy_bytes(), original.as_slice());
    assert_eq!(proposal.state, PolicyMigrationState::Adopted);
}

#[test]
fn capability_limit_tampering_fails_closed() {
    let corpus = MemoryCorpus::with_limits(scope(), 16, 4096).expect("corpus");
    let mut capabilities = corpus.capabilities();
    capabilities.effective_limits.optional_byte_budget = MAX_OPTIONAL_BYTES + 1;
    assert_eq!(
        capabilities.validate(),
        Err(MemoryError::InvalidCapabilities)
    );
}

#[test]
fn capability_descriptor_matches_closed_consumer_schema() {
    let corpus = MemoryCorpus::with_limits(scope(), 16, 4096).expect("corpus");
    let schema: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../contracts/context-memory/capabilities.schema.json"
    ))
    .expect("schema");
    let validator = jsonschema::validator_for(&schema).expect("validator");
    let value = serde_json::to_value(corpus.capabilities()).expect("capabilities");
    assert!(validator.is_valid(&value), "{value}");
}

#[test]
fn exact_extract_review_and_admission_keep_lifecycle_separate() {
    let mut corpus = MemoryCorpus::with_limits(scope(), 16, 4096).expect("corpus");
    let item = source(
        "hist",
        "HP loss was 2, and the action settled.",
        4,
        5,
        false,
    );
    let reference = item.reference();
    corpus.admit(item).expect("admit");
    let proposal = corpus
        .exact_extract(
            "proposal-1",
            std::slice::from_ref(&reference),
            "branch-a",
            10,
            5,
            "2026-09-10T12:00:00Z",
            "2026-09-10T12:00:00Z",
            "2026-09-11T12:00:00Z",
        )
        .expect("extract");
    assert_eq!(proposal.status, ProposalStatus::MachineChecked);
    assert!(!proposal.applied);
    let review = MemoryReview {
        schema: MEMORY_REVIEW_SCHEMA.to_owned(),
        review_id: "review-1".to_owned(),
        proposal_id: proposal.proposal_id.clone(),
        proposal_version: proposal.version,
        proposal_sha256: proposal.sha256.clone(),
        scope: scope(),
        source_manifest_sha256: source_manifest_digest(std::slice::from_ref(&reference)),
        revocation_epoch: corpus.revocation_epoch(),
        reviewer_ref: "independent-reviewer".to_owned(),
        decision: ReviewDecision::Admit,
        support_check: SupportCheck::ExactExtractChecked,
        reason_codes: vec!["critical_facts_preserved".to_owned()],
        created_at: "2026-09-10T12:01:00Z".to_owned(),
        creates_active_revision: false,
    };
    corpus
        .admit_reviewed_proposal(&proposal, &review, "2026-09-10T12:00:00Z")
        .expect("reviewed admission");
    assert!(
        corpus
            .entries()
            .any(|entry| entry.kind == MemoryKind::Summary)
    );
}

#[test]
fn explicit_summary_job_captures_bounded_sources_and_unknown_is_terminal() {
    let mut corpus = MemoryCorpus::with_limits(scope(), 16, 4096).expect("corpus");
    let item = source("hist", "HP loss was 2", 4, 5, false);
    let reference = item.reference();
    corpus.admit(item).expect("admit");
    let mut peer = FakeSummaryPeer::new(b"review me".to_vec()).expect("peer");
    let mut jobs = SummaryJobStore::new(4).expect("jobs");
    let job = SummaryJob {
        schema: MEMORY_JOB_SCHEMA.to_owned(),
        job_id: "job-1".to_owned(),
        scope: scope(),
        branch_id: "branch-a".to_owned(),
        sources: vec![reference],
        cutoff: 10,
        corpus_generation: 5,
        source_manifest_sha256: source_manifest_digest(&[MemoryRef::new(
            "hist",
            1,
            sha256_hex("HP loss was 2"),
        )]),
        generator_profile: "fake-v1".to_owned(),
        generator_prompt_sha256: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            .to_owned(),
        output_schema_sha256: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            .to_owned(),
        idempotency_key: "job-key-1".to_owned(),
        command_window_id: "window-1".to_owned(),
        state: JobState::Queued,
        attempt_id: None,
        provider_write_state: ProviderWriteState::NotStarted,
        input_bytes: 13,
        max_output_bytes: 8192,
        review_required: true,
        auto_apply: false,
        deadline_at: "2026-09-11T12:00:00Z".to_owned(),
        effect_class: "authorized_summary_generation_only".to_owned(),
    };
    jobs.admit(job).expect("job");
    let proposal = jobs
        .execute_explicit("job-1", &corpus, &mut peer, "2026-09-10T12:00:00Z", true)
        .expect("generate");
    assert_eq!(proposal.status, ProposalStatus::ReviewRequired);
    assert_eq!(peer.requests().len(), 1);
    jobs.mark_unknown("job-1", "attempt-unknown")
        .expect("unknown");
    assert_eq!(jobs.retry("job-1"), Err(MemoryError::JobUnknown));
}

#[test]
fn approval_is_held_until_explicit_resume_and_revocation_invalidates_it() {
    let mut store = ApprovalStore::default();
    let selection = SelectionManifest {
        schema: MEMORY_SELECTION_SCHEMA.to_owned(),
        selection_id: "selection-1".to_owned(),
        scope: scope(),
        branch_id: "branch-a".to_owned(),
        policy_id: "policy-1".to_owned(),
        policy_version: 1,
        cutoff: 10,
        corpus_generation: 5,
        revocation_epoch: 0,
        selected_sources: Vec::new(),
        pinned_entry_ids: Vec::new(),
        protected_manifest_sha256: sha256_hex("protected"),
        optional_byte_budget: 8192,
        optional_rendered_bytes: 0,
        whole_rendered_bytes: 9,
        prepared_manifest_sha256: sha256_hex("prepared"),
        whole_tokens: None,
        token_measurement: "unavailable".to_owned(),
        budget_status: "bounded_unknown_total".to_owned(),
        rendered_content_ref: "prepared-1".to_owned(),
        expires_at: "2026-09-11T12:00:00Z".to_owned(),
        phase2_revision_id: "revision-1".to_owned(),
        effect_class: "local_preparation_only".to_owned(),
        phase2_prepared_manifest_sha256:
            "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".to_owned(),
        rendered_bytes: b"prepared".to_vec(),
        exclusions: Vec::new(),
    };
    let approval = store
        .bind(
            "approval-1",
            &selection,
            "preview-1",
            source_manifest_digest(&selection.selected_sources),
        )
        .expect("bind");
    assert_eq!(approval.state, ApprovalState::PreviewReady);
    let held = store
        .commit_held("approval-1", 0, "2026-09-10T12:00:00Z")
        .expect("held");
    assert_eq!(held.state, ApprovalState::CommittedHeld);
    let resumed = store
        .explicit_resume("approval-1", 0, "2026-09-10T12:00:00Z")
        .expect("resume");
    assert_eq!(resumed.gameplay_inference_calls, 1);
    store.invalidate_revoked(1);
    assert_eq!(
        store.approval("approval-1").map(|value| value.state),
        Some(ApprovalState::Stale)
    );
}

#[test]
fn wire_query_is_closed_and_projection_lag_is_typed() {
    let query = MemoryQuery {
        schema: MEMORY_QUERY_SCHEMA.to_owned(),
        query_id: "local-correlation".to_owned(),
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
    let wire = serde_json::to_value(&query).expect("query wire");
    assert!(wire.get("query_id").is_none());
    let mut unknown = wire.clone();
    unknown
        .as_object_mut()
        .expect("object")
        .insert("unexpected".to_owned(), serde_json::json!(true));
    assert!(serde_json::from_value::<MemoryQuery>(unknown).is_err());

    let mut corpus = MemoryCorpus::with_limits(scope(), 4, 1024).expect("corpus");
    corpus
        .admit(source("hist", "settled", 1, 1, false))
        .expect("admit");
    corpus.set_projection_health(false);
    assert_eq!(
        corpus.retrieve(&query, "2026-09-10T12:00:00Z"),
        Err(MemoryError::ProjectionUnavailable)
    );
}

#[test]
fn map_acl_and_public_telemetry_keep_authority_separate() {
    let map: MapBundle = serde_json::from_str(include_str!(
        "../../../fixtures/context-memory/maps/visible-map.json"
    ))
    .expect("map fixture");
    assert!(map.validate(&scope(), 10).is_ok());

    let mut invalid = map.clone();
    invalid.generation = 9;
    assert_eq!(
        invalid.validate(&scope(), 10),
        Err(MemoryError::InvalidEntry)
    );

    let mut authorizer = MemoryAuthorizer::default();
    authorizer.grant("reader", MemoryRole::Read).expect("grant");
    assert!(authorizer.check("reader", MemoryRole::Read).is_ok());
    assert_eq!(
        authorizer.check("reader", MemoryRole::Review),
        Err(MemoryError::PermissionDenied)
    );

    let telemetry = MemoryTelemetry {
        retrieval_queries: 2,
        retained_bytes: 10,
        ..MemoryTelemetry::default()
    };
    let public = telemetry.public_snapshot();
    assert_eq!(public["retrieval_queries"], 2);
    assert!(public["raw_query"].is_null());
    assert!(public["raw_content"].is_null());
}
