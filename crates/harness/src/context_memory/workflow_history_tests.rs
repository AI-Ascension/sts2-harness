// SPDX-License-Identifier: MIT

// Positive and negative fixtures for workflow-scoped history selection, extraction and the
// zero-inference search/preview boundary (issue #113).

#[cfg(test)]
mod workflow_history_tests {
    use super::*;
    use std::collections::BTreeSet;

    const NOW: &str = "2026-09-10T12:00:00Z";

    fn scope() -> MemoryScope {
        MemoryScope::new("project", "run", "episode", "agent")
    }

    fn entry(
        id: &str,
        text: &str,
        kind: MemoryKind,
        observed: u64,
        created: &str,
        expires: &str,
        protected: bool,
    ) -> MemoryEntry {
        MemoryEntry::new(
            scope(),
            id,
            format!("record-{id}"),
            kind,
            if kind == MemoryKind::Summary {
                EvidenceStatus::Derived
            } else {
                EvidenceStatus::Observed
            },
            "branch-a",
            id,
            text.as_bytes().to_vec(),
            observed,
            observed,
            1,
            created,
            expires,
            "synthetic-v1",
            protected,
        )
    }

    fn corpus() -> MemoryCorpus {
        MemoryCorpus::with_limits(scope(), 16, 4096).unwrap_or_else(|_| unreachable!())
    }

    fn policy(mode: PolicyMode, approved: Vec<MemoryRef>) -> MemoryPolicy {
        MemoryPolicy {
            schema: MEMORY_POLICY_SCHEMA.to_owned(),
            policy_id: "policy-1".to_owned(),
            version: 1,
            scope: scope(),
            mode,
            status: PolicyStatus::Approved,
            phase2_revision_id: Some("revision-1".to_owned()),
            corpus_generation: 1,
            rolling_same_episode_sources: false,
            cross_scope: false,
            approved_summary_catalog: approved,
            ranker_version: "lexical-v1".to_owned(),
            query_derivation_version: "manual-v1".to_owned(),
            max_candidates: 8,
            max_results: 8,
            max_selected: 32,
            optional_byte_budget: 8192,
            fallback: SelectionFallback::Block,
            automatic_summary_activation: false,
            generate_during_selection: false,
            authorization_policy_version: "auth-1".to_owned(),
        }
    }

    fn request(
        policy: MemoryPolicy,
        optional: Vec<MemoryRef>,
        pinned: Vec<String>,
    ) -> SelectionRequest {
        SelectionRequest {
            selection_id: "selection-1".to_owned(),
            policy,
            branch_id: "branch-a".to_owned(),
            cutoff: 8,
            corpus_generation: 1,
            mandatory_bytes: b"protected-live-state".to_vec(),
            mandatory_manifest_sha256: sha256_hex("protected-live-state"),
            optional_sources: optional,
            pinned_entry_ids: pinned,
            prepared_content_ref: "prepared-1".to_owned(),
            expires_at: "2026-09-11T12:00:00Z".to_owned(),
            phase2_prepared_manifest_sha256:
                "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd".to_owned(),
        }
    }

    fn session(corpus: MemoryCorpus) -> WorkflowHistorySession {
        WorkflowHistorySession::new(
            WorkflowHistoryOwner::new("workflow-1", scope(), "branch-a"),
            corpus,
        )
        .unwrap_or_else(|_| unreachable!())
    }

    fn summary_job(source: &MemoryRef, input_bytes: usize) -> SummaryJob {
        SummaryJob {
            schema: MEMORY_JOB_SCHEMA.to_owned(),
            job_id: "job-1".to_owned(),
            scope: scope(),
            branch_id: "branch-a".to_owned(),
            sources: vec![source.clone()],
            cutoff: 8,
            corpus_generation: 1,
            source_manifest_sha256: source_manifest_digest(std::slice::from_ref(source)),
            generator_profile: "fake-v1".to_owned(),
            generator_prompt_sha256: sha256_hex("prompt"),
            output_schema_sha256: sha256_hex("schema"),
            idempotency_key: "key-1".to_owned(),
            command_window_id: "window-1".to_owned(),
            state: JobState::Queued,
            attempt_id: None,
            provider_write_state: ProviderWriteState::NotStarted,
            input_bytes,
            max_output_bytes: 256,
            review_required: true,
            auto_apply: false,
            deadline_at: "2026-09-11T12:00:00Z".to_owned(),
            effect_class: "authorized_summary_generation_only".to_owned(),
        }
    }

    fn query() -> MemoryQuery {
        MemoryQuery {
            schema: MEMORY_QUERY_SCHEMA.to_owned(),
            query_id: "query-1".to_owned(),
            scope: scope(),
            branch_id: "branch-a".to_owned(),
            query: "settled action".to_owned(),
            cutoff: 8,
            corpus_generation: 1,
            ranker_version: "lexical-v1".to_owned(),
            limit: 8,
            max_candidates: 8,
            effect_class: "local_read_no_inference".to_owned(),
        }
    }

    #[test]
    fn scoped_selection_and_extraction_are_deterministic_and_preserve_mandatory_and_pins() {
        let mut corpus = corpus();
        let source = entry(
            "hist-1",
            "settled action one",
            MemoryKind::HistoricalObservation,
            1,
            "2026-09-10T10:00:00Z",
            "2026-09-11T12:00:00Z",
            false,
        );
        let reference = source.reference();
        assert!(corpus.admit(source).is_ok());
        let mut session = session(corpus);
        let request = request(
            policy(PolicyMode::ManualSnapshot, Vec::new()),
            vec![reference.clone()],
            vec!["hist-1".to_owned()],
        );
        let (first_receipt, first) = session
            .preview(&request, NOW)
            .unwrap_or_else(|_| unreachable!());
        let (_, second) = session
            .preview(&request, NOW)
            .unwrap_or_else(|_| unreachable!());
        assert_eq!(
            first.prepared_manifest_sha256,
            second.prepared_manifest_sha256
        );
        assert_eq!(first.pinned_entry_ids, vec!["hist-1".to_owned()]);
        assert_eq!(first.selected_sources, vec![reference.clone()]);
        assert_eq!(
            first.protected_manifest_sha256,
            sha256_hex("protected-live-state")
        );
        assert!(first.rendered_bytes.starts_with(b"phase2-memory-v1\n"));
        assert_eq!(first_receipt.operation, WorkflowHistoryOperation::Preview);
        assert_eq!(first_receipt.outcome, ReceiptOutcome::ZeroInference);
        let (_, extract_one) = session
            .extract(
                "proposal-a",
                std::slice::from_ref(&reference),
                8,
                1,
                NOW,
                "2026-09-10T12:00:00Z",
                "2026-09-11T12:00:00Z",
            )
            .unwrap_or_else(|_| unreachable!());
        let (_, extract_two) = session
            .extract(
                "proposal-b",
                &[reference],
                8,
                1,
                NOW,
                "2026-09-10T12:00:00Z",
                "2026-09-11T12:00:00Z",
            )
            .unwrap_or_else(|_| unreachable!());
        assert_eq!(extract_one.sha256, extract_two.sha256);
        assert_eq!(session.count(WorkflowHistoryOperation::Extraction), 2);
    }

    #[test]
    fn ineligible_material_cannot_enter_an_approved_invocation() {
        let mut corpus = corpus();
        let eligible = entry(
            "hist-1",
            "eligible settled action",
            MemoryKind::HistoricalObservation,
            1,
            "2026-09-10T10:00:00Z",
            "2026-09-11T12:00:00Z",
            false,
        );
        let eligible_ref = eligible.reference();
        assert!(corpus.admit(eligible).is_ok());
        assert!(corpus
            .admit(entry(
                "hist-2",
                "expired history",
                MemoryKind::HistoricalObservation,
                1,
                "2026-09-10T09:00:00Z",
                "2026-09-10T11:00:00Z",
                false,
            ))
            .is_ok());
        assert!(corpus
            .admit(entry(
                "hist-3",
                "private history",
                MemoryKind::HistoricalObservation,
                1,
                "2026-09-10T10:00:00Z",
                "2026-09-11T12:00:00Z",
                true,
            ))
            .is_ok());
        assert!(corpus
            .admit(entry(
                "sum-1",
                "unreviewed summary",
                MemoryKind::Summary,
                1,
                "2026-09-10T10:00:00Z",
                "2026-09-11T12:00:00Z",
                false,
            ))
            .is_ok());
        let expired_ref = MemoryRef::new("hist-2", 1, sha256_hex("expired history"));
        let private_ref = MemoryRef::new("hist-3", 1, sha256_hex("private history"));
        let unreviewed_ref = MemoryRef::new("sum-1", 1, sha256_hex("unreviewed summary"));
        let foreign_ref = MemoryRef::new("hist-9", 1, sha256_hex("foreign"));
        let mut session = session(corpus);
        let request = request(
            policy(PolicyMode::ManualSnapshot, Vec::new()),
            vec![
                eligible_ref.clone(),
                expired_ref,
                private_ref,
                foreign_ref,
            ],
            Vec::new(),
        );
        let (_, selection) = session
            .preview(&request, NOW)
            .unwrap_or_else(|_| unreachable!());
        assert_eq!(selection.selected_sources, vec![eligible_ref]);
        let reasons = selection
            .exclusions
            .iter()
            .map(|exclusion| (exclusion.entry_id.clone(), exclusion.reason.clone()))
            .collect::<BTreeSet<_>>();
        assert!(reasons.contains(&("hist-3".to_owned(), "ineligible_source".to_owned())));
        assert!(reasons.contains(&("hist-2".to_owned(), "ineligible_source".to_owned())));
        assert!(reasons.contains(&("hist-9".to_owned(), "missing_source".to_owned())));
        let bounded = SelectionRequest {
            selection_id: "selection-2".to_owned(),
            policy: policy(PolicyMode::BoundedPerDecision, Vec::new()),
            optional_sources: vec![unreviewed_ref],
            ..request
        };
        let (_, bounded_selection) = session
            .preview(&bounded, NOW)
            .unwrap_or_else(|_| unreachable!());
        assert!(bounded_selection.selected_sources.is_empty());
        assert!(bounded_selection
            .exclusions
            .iter()
            .any(|exclusion| exclusion.reason == "catalog_not_approved"));
    }

    #[test]
    fn search_and_preview_perform_zero_inference_measured_at_the_provider_boundary() {
        let mut corpus = corpus();
        let source = entry(
            "hist-1",
            "settled action one",
            MemoryKind::HistoricalObservation,
            1,
            "2026-09-10T10:00:00Z",
            "2026-09-11T12:00:00Z",
            false,
        );
        let reference = source.reference();
        let source_bytes = source.byte_length;
        assert!(corpus.admit(source).is_ok());
        let mut session = session(corpus);
        let mut peer =
            FakeSummaryPeer::new(b"summary output".to_vec()).unwrap_or_else(|_| unreachable!());
        let mut counting = CountingSummaryProvider::new(&mut peer);
        let (search_receipt, response) = session
            .search(&query(), NOW)
            .unwrap_or_else(|_| unreachable!());
        assert_eq!(response.inference_calls, 0);
        assert_eq!(search_receipt.provider_attempts, 0);
        assert_eq!(search_receipt.outcome, ReceiptOutcome::ZeroInference);
        let request = request(
            policy(PolicyMode::ManualSnapshot, Vec::new()),
            vec![reference.clone()],
            Vec::new(),
        );
        let (preview_receipt, _) = session
            .preview(&request, NOW)
            .unwrap_or_else(|_| unreachable!());
        assert_eq!(preview_receipt.provider_attempts, 0);
        assert_eq!(counting.attempts(), 0);
        let mut jobs =
            SummaryJobStore::new(4).unwrap_or_else(|_| unreachable!());
        assert!(jobs
            .admit(summary_job(&reference, source_bytes))
            .unwrap_or_else(|_| unreachable!()));
        let (_, proposal) = session
            .generate(&mut jobs, "job-1", &mut counting, true, NOW)
            .unwrap_or_else(|_| unreachable!());
        assert!(!proposal.content.is_empty());
        assert_eq!(counting.attempts(), 1);
        let (again, _) = session
            .search(&query(), NOW)
            .unwrap_or_else(|_| unreachable!());
        assert_eq!(again.provider_attempts, 0);
        assert_eq!(counting.attempts(), 1);
    }
}
