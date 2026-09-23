// SPDX-License-Identifier: MIT

// Negative fixtures proving a workflow history session cannot address material owned by another
// scope or branch (issue #113).

#[cfg(test)]
mod workflow_history_scope_tests {
    use super::*;

    const NOW: &str = "2026-09-10T12:00:00Z";

    fn scope() -> MemoryScope {
        MemoryScope::new("project", "run", "episode", "agent")
    }

    fn foreign_scope() -> MemoryScope {
        MemoryScope::new("project", "run", "episode", "other-agent")
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

    fn entry(id: &str, scope: MemoryScope) -> MemoryEntry {
        MemoryEntry::new(
            scope,
            id,
            format!("record-{id}"),
            MemoryKind::HistoricalObservation,
            EvidenceStatus::Observed,
            "branch-a",
            id,
            format!("{id} body").into_bytes(),
            1,
            1,
            1,
            "2026-09-10T10:00:00Z",
            "2026-09-11T12:00:00Z",
            "synthetic-v1",
            false,
        )
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

    fn request() -> SelectionRequest {
        SelectionRequest {
            selection_id: "selection-1".to_owned(),
            policy: MemoryPolicy {
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
            },
            branch_id: "branch-a".to_owned(),
            cutoff: 8,
            corpus_generation: 1,
            mandatory_bytes: b"protected-live-state".to_vec(),
            mandatory_manifest_sha256: sha256_hex("protected-live-state"),
            optional_sources: Vec::new(),
            pinned_entry_ids: Vec::new(),
            prepared_content_ref: "prepared-1".to_owned(),
            expires_at: "2026-09-11T12:00:00Z".to_owned(),
            phase2_prepared_manifest_sha256:
                "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd".to_owned(),
        }
    }

    #[test]
    fn a_session_refuses_a_corpus_owned_by_another_scope() {
        let mut foreign = MemoryCorpus::with_limits(foreign_scope(), 16, 4096)
            .unwrap_or_else(|_| unreachable!());
        assert!(foreign.admit(entry("hist-foreign", foreign_scope())).is_ok());
        assert_eq!(
            WorkflowHistorySession::new(
                WorkflowHistoryOwner::new("workflow-1", scope(), "branch-a"),
                foreign,
            )
            .err(),
            Some(MemoryError::PermissionDenied)
        );
    }

    #[test]
    fn a_session_refuses_foreign_scope_queries_and_foreign_branch_previews() {
        let mut corpus = corpus();
        assert!(corpus.admit(entry("hist-1", scope())).is_ok());
        let mut session = session(corpus);
        let mut foreign_query = query();
        foreign_query.scope = foreign_scope();
        assert_eq!(
            session.search(&foreign_query, NOW).err(),
            Some(MemoryError::PermissionDenied)
        );
        let foreign_branch = SelectionRequest {
            branch_id: "branch-b".to_owned(),
            ..request()
        };
        assert_eq!(
            session.preview(&foreign_branch, NOW).err(),
            Some(MemoryError::PermissionDenied)
        );
    }
}
