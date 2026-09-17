// SPDX-License-Identifier: MIT

#[cfg(test)]
mod resume_tests {
    use super::*;

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

    fn policy(generation: u64) -> MemoryPolicy {
        MemoryPolicy {
            schema: MEMORY_POLICY_SCHEMA.to_owned(),
            policy_id: "policy-1".to_owned(),
            version: 1,
            scope: scope(),
            mode: PolicyMode::ManualSnapshot,
            status: PolicyStatus::Approved,
            phase2_revision_id: Some("revision-1".to_owned()),
            corpus_generation: generation,
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
        }
    }

    fn query(generation: u64) -> MemoryQuery {
        MemoryQuery {
            schema: MEMORY_QUERY_SCHEMA.to_owned(),
            query_id: "query-1".to_owned(),
            scope: scope(),
            branch_id: "branch-a".to_owned(),
            query: "optional".to_owned(),
            cutoff: 2,
            corpus_generation: generation,
            ranker_version: "lexical-v1".to_owned(),
            limit: 8,
            max_candidates: 64,
            effect_class: "local_read_no_inference".to_owned(),
        }
    }

    /// Build one held approval whose prepared input contains `marker` and return the ledger, the
    /// held approval and the exact prepared bytes the caller must submit.
    fn held_resume(marker: &str) -> (FirstResumeLedger, MemoryApproval, Vec<u8>) {
        let mut corpus =
            MemoryCorpus::with_limits(scope(), 8, 4096).unwrap_or_else(|_| unreachable!());
        assert_eq!(
            corpus.admit(entry("source", "optional history")),
            Ok(AdmissionOutcome::Inserted)
        );
        let realized = realize_policy(
            &corpus,
            &policy(corpus.generation()),
            "selection-1",
            &query(corpus.generation()),
            marker.as_bytes().to_vec(),
            sha256_hex("phase2"),
            "prepared-1",
            "2026-09-11T12:00:00Z",
            "2026-09-10T12:00:00Z",
        )
        .unwrap_or_else(|_| unreachable!());
        let mut approvals = ApprovalStore::default();
        approvals
            .bind(
                "approval-1",
                &realized.selection,
                "preview-1",
                source_manifest_digest(&realized.selection.selected_sources),
            )
            .unwrap_or_else(|_| unreachable!());
        let held = approvals
            .commit_held("approval-1", 0, "2026-09-10T12:00:00Z")
            .unwrap_or_else(|_| unreachable!());
        let mut ledger = FirstResumeLedger::default();
        ledger
            .prepare(&held, &realized.selection)
            .unwrap_or_else(|_| unreachable!());
        (ledger, held, realized.selection.rendered_bytes)
    }

    // Harness #243 regression. These three types derived `Debug` over the prepared-input bytes
    // that `#[serde(skip)]` keeps out of serialized output; serde attributes do not apply to
    // Debug, so the derived formatting published them anyway. Both ordinary and alternate
    // formatting must withhold the bytes while the allowlisted shape survives, and the fixture
    // must positively contain them so the assertions cannot pass vacuously.
    #[test]
    fn resume_debug_withholds_prepared_bytes_from_every_type() {
        // Original MIT-licensed synthetic fixture; no provider, credential, or user data.
        let marker = "resume-debug-private-marker";
        let (mut ledger, held, prepared_bytes) = held_resume(marker);
        let (outcome, submission) = ledger
            .submit_first(&held, 0, "2026-09-10T12:00:00Z", &prepared_bytes)
            .unwrap_or_else(|_| unreachable!());
        assert_eq!(outcome, ResumeOutcome::Submitted);
        assert_eq!(submission.rendered_bytes, prepared_bytes);

        let prepared = ledger
            .prepared
            .get("approval-1")
            .unwrap_or_else(|| unreachable!());
        assert!(!prepared.rendered_bytes.is_empty());
        assert!(String::from_utf8_lossy(&prepared.rendered_bytes).contains(marker));
        assert!(String::from_utf8_lossy(&submission.rendered_bytes).contains(marker));
        let serialized = serde_json::to_vec(prepared).unwrap_or_else(|_| unreachable!());
        assert!(
            !String::from_utf8_lossy(&serialized).contains(marker),
            "serialization already withholds the prepared bytes"
        );

        for formatted in [format!("{prepared:?}"), format!("{prepared:#?}")] {
            assert_withheld(&formatted, &prepared.rendered_bytes, marker);
            assert!(formatted.contains("PreparedResume"));
            assert!(formatted.contains(&format!(
                "rendered_byte_count: {}",
                prepared.rendered_bytes.len()
            )));
        }
        for formatted in [format!("{ledger:?}"), format!("{ledger:#?}")] {
            assert_withheld(&formatted, &prepared.rendered_bytes, marker);
            assert!(formatted.contains("FirstResumeLedger"));
            assert!(formatted.contains("prepared_count: 1"));
        }
        for formatted in [format!("{submission:?}"), format!("{submission:#?}")] {
            assert_withheld(&formatted, &submission.rendered_bytes, marker);
            assert!(formatted.contains("ResumeSubmission"));
        }

        // Serialization, equality and consumption semantics are unchanged by the Debug work.
        assert_eq!(
            serialized,
            serde_json::to_vec(prepared).unwrap_or_else(|_| unreachable!())
        );
        assert_eq!(prepared, &prepared.clone());
        assert_eq!(submission, submission.clone());
        assert!(ledger.is_consumed("approval-1"));
    }

    fn assert_withheld(formatted: &str, bytes: &[u8], marker: &str) {
        let compact: String = formatted
            .chars()
            .filter(|item| !item.is_whitespace())
            .collect();
        for representation in [
            String::from_utf8_lossy(bytes).into_owned(),
            format!("{bytes:?}"),
            format!("{bytes:#?}"),
        ] {
            let representation: String = representation
                .chars()
                .filter(|item| !item.is_whitespace())
                .collect();
            assert!(
                !compact.contains(representation.as_str()),
                "debug published the prepared bytes"
            );
        }
        assert!(
            !compact.contains(marker),
            "debug published caller-visible prepared content"
        );
    }
}
