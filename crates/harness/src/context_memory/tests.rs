// SPDX-License-Identifier: MIT

#[cfg(test)]
mod tests {
    use super::*;

    fn scope() -> MemoryScope {
        MemoryScope::new("project", "run", "episode", "agent")
    }

    fn entry(id: &str, text: &str, observed: u64, admitted: u64, generation: u64) -> MemoryEntry {
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
            admitted,
            generation,
            "2026-09-10T10:00:00Z",
            "2026-09-11T12:00:00Z",
            "synthetic-v1",
            false,
        )
    }

    fn corpus() -> MemoryCorpus {
        MemoryCorpus::with_limits(scope(), 8, 1024).unwrap_or_else(|_| unreachable!())
    }

    #[test]
    fn source_manifest_uses_canonical_json_bytes() {
        let refs = vec![
            MemoryRef::new(
                "hist-1",
                1,
                "ce08b83db41f079414c5ccec002eb00c1cfb2a550ca263089437a2fb444bf1a3",
            ),
            MemoryRef::new(
                "hist-2",
                1,
                "f2426618c5f5257aa779a7191f54e39f04d51b855f0279f75c1803c34b22ac04",
            ),
        ];
        assert_eq!(
            source_manifest_digest(&refs),
            "c466563953256d06c377e2922d23358bd11d38048a59e1fa070d01080edc5d10"
        );
    }

    #[test]
    fn causal_filtering_and_ties_are_deterministic() {
        let mut corpus = corpus();
        assert_eq!(
            corpus.admit(entry("b", "HP action settled", 4, 5, 1)),
            Ok(AdmissionOutcome::Inserted)
        );
        assert_eq!(
            corpus.admit(entry("a", "HP action settled", 4, 5, 1)),
            Ok(AdmissionOutcome::Inserted)
        );
        assert_eq!(
            corpus.admit(entry("future", "HP action settled", 12, 12, 12)),
            Ok(AdmissionOutcome::Inserted)
        );
        let query = MemoryQuery {
            schema: MEMORY_QUERY_SCHEMA.to_owned(),
            query_id: "query-1".to_owned(),
            scope: scope(),
            branch_id: "branch-a".to_owned(),
            query: "HP action settled".to_owned(),
            cutoff: 10,
            corpus_generation: 3,
            ranker_version: "lexical-v1".to_owned(),
            limit: 8,
            max_candidates: 64,
            effect_class: "local_read_no_inference".to_owned(),
        };
        let response = corpus
            .retrieve(&query, "2026-09-10T12:00:00Z")
            .unwrap_or_else(|_| unreachable!());
        assert_eq!(
            response
                .results
                .iter()
                .map(|result| result.source.entry_id.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "b"]
        );
        assert_eq!(response.inference_calls, 0);
    }

    #[test]
    fn changed_identity_conflicts_and_publication_is_atomic() {
        let mut corpus = corpus();
        let first = entry("same", "one", 1, 1, 1);
        assert_eq!(corpus.admit(first.clone()), Ok(AdmissionOutcome::Inserted));
        assert_eq!(corpus.admit(first), Ok(AdmissionOutcome::Duplicate));
        assert_eq!(
            corpus.admit(entry("same", "two", 1, 1, 1)),
            Err(MemoryError::Conflict)
        );
        corpus.set_failpoint(Some(PublicationFailpoint::BeforeMetadata));
        assert_eq!(
            corpus.admit(entry("failed", "three", 1, 1, 1)),
            Err(MemoryError::PublicationFailed)
        );
        assert!(
            corpus
                .entry(&MemoryRef::new("failed", 1, sha256_hex("three")))
                .is_none()
        );
    }

    #[test]
    fn revocation_fences_dependents_before_cleanup() {
        let mut corpus = corpus();
        let root = entry("root", "settled action", 1, 1, 1);
        let root_ref = root.reference();
        assert_eq!(corpus.admit(root), Ok(AdmissionOutcome::Inserted));
        let mut derived = entry("derived", "settled action", 2, 2, 2);
        derived.kind = MemoryKind::Extract;
        derived.evidence = EvidenceStatus::Derived;
        derived.parents = vec![MemoryParent::from(&root_ref)];
        derived.lineage_depth = 1;
        assert_eq!(
            corpus.admit(derived.clone()),
            Ok(AdmissionOutcome::Inserted)
        );
        let query = MemoryQuery {
            schema: MEMORY_QUERY_SCHEMA.to_owned(),
            query_id: "query-1".to_owned(),
            scope: scope(),
            branch_id: "branch-a".to_owned(),
            query: "settled".to_owned(),
            cutoff: 3,
            corpus_generation: corpus.generation(),
            ranker_version: "lexical-v1".to_owned(),
            limit: 8,
            max_candidates: 64,
            effect_class: "local_read_no_inference".to_owned(),
        };
        assert_eq!(
            corpus
                .revoke(&[root_ref], "2026-09-10T12:00:00Z")
                .map(|record| record.denial_committed),
            Ok(true)
        );
        let response = corpus
            .retrieve(&query, "2026-09-10T12:00:00Z")
            .unwrap_or_else(|_| unreachable!());
        assert!(response.results.is_empty());
        assert_eq!(corpus.cleanup_revoked(), 2);
        assert_eq!(
            corpus.entry(&derived.reference()).map(|entry| entry.status),
            Some(EntryStatus::Revoked)
        );
    }

    #[test]
    fn extractive_claims_require_exact_source_bytes() {
        let mut corpus = corpus();
        let source = entry("source", "HP loss was 2, and the action settled.", 1, 1, 1);
        let reference = source.reference();
        assert_eq!(corpus.admit(source), Ok(AdmissionOutcome::Inserted));
        let bytes = b"HP loss was 2, and the action settled.".to_vec();
        let proposal = MemoryProposal {
            schema: MEMORY_PROPOSAL_SCHEMA.to_owned(),
            proposal_id: "proposal-1".to_owned(),
            version: 1,
            scope: scope(),
            branch_id: "branch-a".to_owned(),
            kind: ProposalKind::Extractive,
            sources: vec![reference.clone()],
            cutoff: 2,
            corpus_generation: corpus.generation(),
            claims: vec![MemoryClaim {
                claim_id: "claim-1".to_owned(),
                text: String::from_utf8(bytes.clone()).unwrap_or_default(),
                support: ClaimSupport::Extractive,
                citations: vec![Citation {
                    source: reference,
                    start_byte: 0,
                    end_byte: bytes.len(),
                    quote_sha256: sha256_hex(&bytes),
                }],
                uncertainty: "historical".to_owned(),
                applicability: Applicability::Historical,
            }],
            omissions: Vec::new(),
            contradictions: Vec::new(),
            lineage_depth: 1,
            status: ProposalStatus::MachineChecked,
            content_ref: "proposal-1".to_owned(),
            sha256: sha256_hex("summary"),
            byte_length: 7,
            source_reconstruction: SourceReconstruction::Available,
            created_at: "2026-09-10T11:00:00Z".to_owned(),
            expires_at: "2026-09-11T12:00:00Z".to_owned(),
            applied: false,
            content: b"summary".to_vec(),
        };
        assert!(
            proposal
                .validate_against(&corpus, "2026-09-10T12:00:00Z")
                .is_ok()
        );
    }

    #[test]
    fn unknown_summary_outcome_is_not_retried() {
        let mut jobs = SummaryJobStore::new(2).unwrap_or_else(|_| unreachable!());
        let job = SummaryJob {
            schema: MEMORY_JOB_SCHEMA.to_owned(),
            job_id: "job-1".to_owned(),
            scope: scope(),
            branch_id: "branch-a".to_owned(),
            sources: vec![MemoryRef::new(
                "source",
                1,
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )],
            cutoff: 1,
            corpus_generation: 1,
            source_manifest_sha256:
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
            generator_profile: "fake-v1".to_owned(),
            generator_prompt_sha256:
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned(),
            output_schema_sha256:
                "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".to_owned(),
            idempotency_key: "key-1".to_owned(),
            command_window_id: "window-1".to_owned(),
            state: JobState::Queued,
            attempt_id: None,
            provider_write_state: ProviderWriteState::NotStarted,
            input_bytes: 10,
            max_output_bytes: 8192,
            review_required: true,
            auto_apply: false,
            deadline_at: "2026-09-11T12:00:00Z".to_owned(),
            effect_class: "authorized_summary_generation_only".to_owned(),
        };
        assert_eq!(jobs.admit(job), Ok(true));
        assert!(jobs.mark_unknown("job-1", "attempt-1").is_ok());
        assert_eq!(jobs.retry("job-1"), Err(MemoryError::JobUnknown));
    }

    #[test]
    fn selection_preserves_pins_and_measures_whole_render() {
        let mut corpus = corpus();
        let source = entry("source", "optional history", 1, 1, 1);
        let reference = source.reference();
        assert_eq!(corpus.admit(source), Ok(AdmissionOutcome::Inserted));
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
            query_derivation_version: "manual-v1".to_owned(),
            max_candidates: 64,
            max_results: 8,
            max_selected: 32,
            optional_byte_budget: 8192,
            fallback: SelectionFallback::Block,
            automatic_summary_activation: false,
            generate_during_selection: false,
            authorization_policy_version: "auth-1".to_owned(),
        };
        let request = SelectionRequest {
            selection_id: "selection-1".to_owned(),
            policy,
            branch_id: "branch-a".to_owned(),
            cutoff: 2,
            corpus_generation: corpus.generation(),
            mandatory_bytes: b"protected".to_vec(),
            mandatory_manifest_sha256: sha256_hex("protected"),
            optional_sources: vec![reference],
            pinned_entry_ids: vec!["source".to_owned()],
            prepared_content_ref: "prepared-1".to_owned(),
            expires_at: "2026-09-11T12:00:00Z".to_owned(),
            phase2_prepared_manifest_sha256:
                "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd".to_owned(),
        };
        let selection = corpus
            .select(&request, "2026-09-10T12:00:00Z")
            .unwrap_or_else(|_| unreachable!());
        assert_eq!(selection.optional_rendered_bytes, b"optional history".len());
        assert!(selection.whole_rendered_bytes > selection.optional_rendered_bytes);
        assert_eq!(selection.token_measurement, "unavailable");
    }
}
