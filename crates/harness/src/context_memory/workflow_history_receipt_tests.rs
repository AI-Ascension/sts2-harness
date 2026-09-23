// SPDX-License-Identifier: MIT

// Positive and negative fixtures for explicit generation authority, lost-reply/restart
// recovery, and the independent review/adoption/commit/resume receipts (issue #113).

#[cfg(test)]
mod workflow_history_receipt_tests {
    use super::*;

    const NOW: &str = "2026-09-10T12:00:00Z";

    fn scope() -> MemoryScope {
        MemoryScope::new("project", "run", "episode", "agent")
    }

    fn entry(id: &str, text: &str) -> MemoryEntry {
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
            "2026-09-10T10:00:00Z",
            "2026-09-11T12:00:00Z",
            "synthetic-v1",
            false,
        )
    }

    fn corpus() -> MemoryCorpus {
        MemoryCorpus::with_limits(scope(), 16, 4096).unwrap_or_else(|_| unreachable!())
    }

    fn session(corpus: MemoryCorpus) -> WorkflowHistorySession {
        WorkflowHistorySession::new(
            WorkflowHistoryOwner::new("workflow-1", scope(), "branch-a"),
            corpus,
        )
        .unwrap_or_else(|_| unreachable!())
    }

    fn policy() -> MemoryPolicy {
        MemoryPolicy {
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

    fn request(optional: Vec<MemoryRef>) -> SelectionRequest {
        SelectionRequest {
            selection_id: "selection-1".to_owned(),
            policy: policy(),
            branch_id: "branch-a".to_owned(),
            cutoff: 8,
            corpus_generation: 1,
            mandatory_bytes: b"protected-live-state".to_vec(),
            mandatory_manifest_sha256: sha256_hex("protected-live-state"),
            optional_sources: optional,
            pinned_entry_ids: Vec::new(),
            prepared_content_ref: "prepared-1".to_owned(),
            expires_at: "2026-09-11T12:00:00Z".to_owned(),
            phase2_prepared_manifest_sha256:
                "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd".to_owned(),
        }
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
            source_manifest_sha256: source_manifest_digest(&[source.clone()]),
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

    fn review(proposal: &MemoryProposal) -> MemoryReview {
        MemoryReview {
            schema: MEMORY_REVIEW_SCHEMA.to_owned(),
            review_id: "review-1".to_owned(),
            proposal_id: proposal.proposal_id.clone(),
            proposal_version: proposal.version,
            proposal_sha256: proposal.sha256.clone(),
            scope: proposal.scope.clone(),
            source_manifest_sha256: source_manifest_digest(&proposal.sources),
            revocation_epoch: 0,
            reviewer_ref: "reviewer-1".to_owned(),
            decision: ReviewDecision::Admit,
            support_check: SupportCheck::IndependentReview,
            reason_codes: vec!["independent_review".to_owned()],
            created_at: "2026-09-10T11:00:00Z".to_owned(),
            creates_active_revision: false,
        }
    }

    struct FailingProvider {
        calls: u32,
    }

    impl SummaryProvider for FailingProvider {
        fn generate(
            &mut self,
            _request: SummaryGenerationRequest,
        ) -> Result<SummaryGeneration, MemoryError> {
            self.calls = self.calls.saturating_add(1);
            Err(MemoryError::Unsupported)
        }
    }

    #[test]
    fn explicit_generation_records_separate_authority_and_bounded_costs() {
        let mut corpus = corpus();
        let source = entry("hist-1", "settled action one");
        let reference = source.reference();
        let source_bytes = source.byte_length;
        assert!(corpus.admit(source).is_ok());
        let mut session = session(corpus);
        let mut jobs = SummaryJobStore::new(4).unwrap_or_else(|_| unreachable!());
        assert!(jobs
            .admit(summary_job(&reference, source_bytes))
            .unwrap_or_else(|_| unreachable!()));
        let mut peer =
            FakeSummaryPeer::new(b"summary output".to_vec()).unwrap_or_else(|_| unreachable!());
        let mut counting = CountingSummaryProvider::new(&mut peer);
        assert_eq!(
            session.generate(&mut jobs, "job-1", &mut counting, false, NOW),
            Err(MemoryError::PermissionDenied)
        );
        assert_eq!(counting.attempts(), 0);
        assert_eq!(session.count(WorkflowHistoryOperation::Generation), 0);
        assert_eq!(
            jobs.job("job-1").map(|job| job.state),
            Some(JobState::Queued)
        );
        let (receipt, proposal) = session
            .generate(&mut jobs, "job-1", &mut counting, true, NOW)
            .unwrap_or_else(|_| unreachable!());
        assert_eq!(receipt.operation, WorkflowHistoryOperation::Generation);
        assert_eq!(receipt.outcome, ReceiptOutcome::Completed);
        assert_eq!(receipt.provider_attempts, 1);
        assert_eq!(receipt.inference_calls, 1);
        assert_eq!(counting.attempts(), 1);
        assert_eq!(
            jobs.job("job-1").map(|job| job.state),
            Some(JobState::Succeeded)
        );
        assert!(proposal.byte_length <= 256);
        assert!(receipt.validate().is_ok());
    }

    #[test]
    fn lost_generation_reply_and_restart_preserve_outcome_unknown_without_duplicate_calls() {
        let mut live_corpus = corpus();
        let source = entry("hist-1", "settled action one");
        let reference = source.reference();
        let source_bytes = source.byte_length;
        assert!(live_corpus.admit(source).is_ok());
        let mut live = session(live_corpus);
        let mut jobs = SummaryJobStore::new(4).unwrap_or_else(|_| unreachable!());
        assert!(jobs
            .admit(summary_job(&reference, source_bytes))
            .unwrap_or_else(|_| unreachable!()));
        let mut failing = FailingProvider { calls: 0 };
        let mut counting = CountingSummaryProvider::new(&mut failing);
        assert_eq!(
            live.generate(&mut jobs, "job-1", &mut counting, true, NOW),
            Err(MemoryError::Unsupported)
        );
        assert_eq!(counting.attempts(), 1);
        assert_eq!(
            jobs.job("job-1").map(|job| job.state),
            Some(JobState::OutcomeUnknown)
        );
        let lost = live
            .receipts()
            .last()
            .cloned()
            .unwrap_or_else(|| unreachable!());
        assert_eq!(lost.outcome, ReceiptOutcome::OutcomeUnknown);
        assert_eq!(lost.provider_attempts, 1);
        assert_eq!(
            live.generate(&mut jobs, "job-1", &mut counting, true, NOW),
            Err(MemoryError::JobUnknown)
        );
        assert_eq!(counting.attempts(), 1);
        assert_eq!(failing.calls, 1);
        assert_eq!(live.count(WorkflowHistoryOperation::Generation), 1);
        let mut restarted_corpus = corpus();
        assert!(restarted_corpus.admit(entry("hist-1", "settled action one")).is_ok());
        let mut restarted = session(restarted_corpus);
        let mut failing_after = FailingProvider { calls: 0 };
        let mut counting_after = CountingSummaryProvider::new(&mut failing_after);
        assert_eq!(
            restarted.generate(&mut jobs, "job-1", &mut counting_after, true, NOW),
            Err(MemoryError::JobUnknown)
        );
        assert_eq!(counting_after.attempts(), 0);
        assert_eq!(failing_after.calls, 0);
        assert_eq!(restarted.count(WorkflowHistoryOperation::Generation), 0);
        assert_eq!(
            jobs.job("job-1").map(|job| job.state),
            Some(JobState::OutcomeUnknown)
        );
        assert_eq!(jobs.retry("job-1"), Err(MemoryError::JobUnknown));
    }

    #[test]
    fn review_adoption_commit_and_resume_remain_independent_receipts() {
        let mut corpus = corpus();
        let source = entry("hist-1", "settled action one");
        let reference = source.reference();
        assert!(corpus.admit(source).is_ok());
        let mut session = session(corpus);
        let (selection_receipt, selection) = session
            .preview(&request(vec![reference.clone()]), NOW)
            .unwrap_or_else(|_| unreachable!());
        assert_eq!(selection_receipt.operation, WorkflowHistoryOperation::Preview);
        let (extraction_receipt, proposal) = session
            .extract(
                "proposal-1",
                &[reference],
                8,
                1,
                NOW,
                "2026-09-10T12:00:00Z",
                "2026-09-11T12:00:00Z",
            )
            .unwrap_or_else(|_| unreachable!());
        assert_eq!(
            extraction_receipt.operation,
            WorkflowHistoryOperation::Extraction
        );
        let review = review(&proposal);
        let mut approvals = ApprovalStore::new();
        let mut resumes = FirstResumeLedger::default();
        assert_eq!(
            session.resume(&mut resumes, &approvals, "approval-1", 0, NOW, b"x"),
            Err(MemoryError::PermissionDenied)
        );
        assert_eq!(
            session.adopt(&proposal, &review, NOW).err(),
            Some(MemoryError::PermissionDenied)
        );
        let review_receipt = session
            .review(&review, &proposal, NOW)
            .unwrap_or_else(|_| unreachable!());
        assert_eq!(review_receipt.operation, WorkflowHistoryOperation::Review);
        assert_eq!(review_receipt.outcome, ReceiptOutcome::Completed);
        assert_eq!(review_receipt.provider_attempts, 0);
        let (adoption_receipt, outcome) = session
            .adopt(&proposal, &review, NOW)
            .unwrap_or_else(|_| unreachable!());
        assert_eq!(outcome, AdmissionOutcome::Inserted);
        assert_eq!(adoption_receipt.operation, WorkflowHistoryOperation::Adoption);
        approvals
            .bind(
                "approval-1",
                &selection,
                "preview-1",
                source_manifest_digest(&selection.selected_sources),
            )
            .unwrap_or_else(|_| unreachable!());
        assert_eq!(
            session.resume(&mut resumes, &approvals, "approval-1", 0, NOW, &selection.rendered_bytes),
            Err(MemoryError::PermissionDenied)
        );
        let (commit_receipt, committed) = session
            .commit_held(&mut approvals, "approval-1", 0, NOW)
            .unwrap_or_else(|_| unreachable!());
        assert_eq!(commit_receipt.operation, WorkflowHistoryOperation::CommitHeld);
        assert_eq!(committed.state, ApprovalState::CommittedHeld);
        resumes
            .prepare(&committed, &selection)
            .unwrap_or_else(|_| unreachable!());
        let (resume_receipt, _) = session
            .resume(
                &mut resumes,
                &approvals,
                "approval-1",
                0,
                NOW,
                &selection.rendered_bytes,
            )
            .unwrap_or_else(|_| unreachable!());
        assert_eq!(resume_receipt.operation, WorkflowHistoryOperation::Resume);
        assert_eq!(resume_receipt.outcome, ReceiptOutcome::Completed);
        let (repeat_receipt, _) = session
            .resume(
                &mut resumes,
                &approvals,
                "approval-1",
                0,
                NOW,
                &selection.rendered_bytes,
            )
            .unwrap_or_else(|_| unreachable!());
        assert_eq!(repeat_receipt.outcome, ReceiptOutcome::Refused);
        assert_eq!(session.count(WorkflowHistoryOperation::Review), 1);
        assert_eq!(session.count(WorkflowHistoryOperation::Adoption), 1);
        assert_eq!(session.count(WorkflowHistoryOperation::CommitHeld), 1);
        assert_eq!(session.count(WorkflowHistoryOperation::Resume), 2);
    }

    #[test]
    fn reviewed_summarization_adopts_derived_memory_without_mutating_the_source() {
        let mut corpus = corpus();
        let source = entry("hist-1", "settled action one");
        let reference = source.reference();
        assert!(corpus.admit(source).is_ok());
        let before = corpus
            .entry(&reference)
            .map(|entry| (entry.sha256.clone(), entry.byte_length, entry.content.clone()))
            .unwrap_or_else(|| unreachable!());
        let before_len = corpus.entries().count();
        let mut session = session(corpus);
        let (_, proposal) = session
            .extract(
                "proposal-1",
                &[reference.clone()],
                8,
                1,
                NOW,
                "2026-09-10T12:00:00Z",
                "2026-09-11T12:00:00Z",
            )
            .unwrap_or_else(|_| unreachable!());
        let review = review(&proposal);
        session
            .review(&review, &proposal, NOW)
            .unwrap_or_else(|_| unreachable!());
        let (_, outcome) = session
            .adopt(&proposal, &review, NOW)
            .unwrap_or_else(|_| unreachable!());
        assert_eq!(outcome, AdmissionOutcome::Inserted);
        let after = session
            .corpus()
            .entry(&reference)
            .map(|entry| (entry.sha256.clone(), entry.byte_length, entry.content.clone()))
            .unwrap_or_else(|| unreachable!());
        assert_eq!(before, after, "adoption must not mutate the source history");
        assert_eq!(session.corpus().entries().count(), before_len + 1);
        let derived = session
            .corpus()
            .entries()
            .find(|entry| entry.entry_id == proposal.proposal_id)
            .unwrap_or_else(|| unreachable!());
        assert_eq!(derived.kind, MemoryKind::Summary);
        assert_eq!(derived.evidence, EvidenceStatus::Derived);
        assert_eq!(derived.parents.len(), proposal.sources.len());
    }
}
