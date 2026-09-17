// SPDX-License-Identifier: MIT

// `#[serde(skip)]` governs serialization only. These assertions pin the redaction of every skipped
// sensitive field: both ordinary and alternate formatting must withhold the field's bytes and its
// raw name, while still emitting a positive control so the test cannot pass vacuously. Serialization
// must remain unaffected. The module shares the parent namespace, so private fields are set here
// directly. See issue #247.
#[cfg(test)]
mod debug_redaction_tests {
    #![allow(clippy::expect_used)]

    use super::*;

    const MARKER: &str = "LEAK-MARKER-0123456789";

    fn scope() -> MemoryScope {
        MemoryScope::new("project", "run", "episode", "agent")
    }

    fn entry(marker: &str) -> MemoryEntry {
        MemoryEntry::new(
            scope(),
            "entry-1",
            "record-1",
            MemoryKind::HistoricalObservation,
            EvidenceStatus::Observed,
            "branch-a",
            "content-ref",
            marker.as_bytes().to_vec(),
            1,
            1,
            1,
            "2026-09-10T10:00:00Z",
            "2026-09-11T12:00:00Z",
            "synthetic-v1",
            true,
        )
    }

    fn exclusion() -> ExclusionReason {
        ExclusionReason {
            entry_id: "entry-2".to_owned(),
            reason: MARKER.to_owned(),
        }
    }

    // `format!("{value:?}")` and `format!("{value:#?}")` must both withhold `forbidden` and the raw
    // skipped field name, and must both contain every `control` fragment (type name + a non-sensitive
    // shape field) so an empty or truncated impl cannot satisfy the test.
    // A leaked `Vec<u8>` renders as a numeric array, e.g. `[76, 69, 65, ...]`, so the raw marker
    // string alone would not detect it. `byte_array` is the exact rendering a leak would produce.
    fn assert_redacted<T: std::fmt::Debug>(value: &T, forbidden: &[&str], controls: &[&str]) {
        for rendered in [format!("{value:?}"), format!("{value:#?}")] {
            for fragment in controls {
                assert!(
                    rendered.contains(fragment),
                    "missing positive control {fragment:?} in {rendered}"
                );
            }
            for fragment in forbidden {
                assert!(
                    !rendered.contains(fragment),
                    "formatting leaked {fragment:?} in {rendered}"
                );
            }
        }
    }

    fn byte_array(marker: &str) -> String {
        format!("{:?}", marker.as_bytes())
    }

    // Serialization must be unchanged: the skipped field still never reaches JSON.
    fn assert_not_serialized<T: serde::Serialize>(value: &T) {
        let json = serde_json::to_string(value).expect("serialize");
        assert!(!json.contains(MARKER), "serialization leaked marker: {json}");
    }

    #[test]
    fn memory_entry_debug_withholds_content() {
        let value = entry(MARKER);
        let leaked = byte_array(MARKER);
        assert_not_serialized(&value);
        assert_redacted(
            &value,
            &[MARKER, leaked.as_str()],
            &["MemoryEntry", "content_len", "entry_id"],
        );
    }

    #[test]
    fn memory_backup_entry_debug_withholds_content() {
        let value = MemoryBackupEntry {
            metadata: entry("unrelated"),
            content: MARKER.as_bytes().to_vec(),
            protected: true,
        };
        let leaked = byte_array(MARKER);
        assert_not_serialized(&value);
        assert_redacted(
            &value,
            &[MARKER, leaked.as_str()],
            &["MemoryBackupEntry", "content_len"],
        );
    }

    #[test]
    fn map_attachment_debug_withholds_bytes() {
        let value = MapAttachment::new(
            MapAttachmentKind::Graph,
            "artifact-1",
            1,
            MARKER.as_bytes().to_vec(),
            Some("application/json".to_owned()),
        );
        let leaked = byte_array(MARKER);
        assert_not_serialized(&value);
        assert_redacted(
            &value,
            &[MARKER, leaked.as_str()],
            &["MapAttachment", "bytes_len", "artifact_id"],
        );
    }

    #[test]
    fn selection_manifest_debug_withholds_rendered_bytes_and_exclusions() {
        let value = SelectionManifest {
            schema: MEMORY_SELECTION_SCHEMA.to_owned(),
            selection_id: "selection-1".to_owned(),
            scope: scope(),
            branch_id: "branch-a".to_owned(),
            policy_id: "policy-1".to_owned(),
            policy_version: 1,
            cutoff: 10,
            corpus_generation: 1,
            revocation_epoch: 0,
            selected_sources: Vec::new(),
            pinned_entry_ids: Vec::new(),
            protected_manifest_sha256: sha256_hex(b"protected"),
            optional_byte_budget: 128,
            optional_rendered_bytes: 0,
            whole_rendered_bytes: MARKER.len(),
            prepared_manifest_sha256: sha256_hex(MARKER.as_bytes()),
            whole_tokens: None,
            token_measurement: "unavailable".to_owned(),
            budget_status: "bounded_unknown_total".to_owned(),
            rendered_content_ref: "prepared-1".to_owned(),
            expires_at: "2026-09-11T12:00:00Z".to_owned(),
            phase2_revision_id: "revision-1".to_owned(),
            effect_class: "local_preparation_only".to_owned(),
            phase2_prepared_manifest_sha256: sha256_hex(b"phase2"),
            rendered_bytes: MARKER.as_bytes().to_vec(),
            exclusions: vec![exclusion()],
        };
        let leaked = byte_array(MARKER);
        assert_not_serialized(&value);
        assert_redacted(
            &value,
            &[MARKER, leaked.as_str(), "reason:"],
            &[
                "SelectionManifest",
                "rendered_bytes_len",
                "exclusion_count",
            ],
        );
    }

    #[test]
    fn retrieval_result_debug_withholds_snippet() {
        let value = RetrievalResult {
            source: MemoryRef::new("entry-1", 1, sha256_hex(b"entry")),
            score: 7,
            reasons: vec!["lexical".to_owned()],
            snippet: Some(MARKER.to_owned()),
        };
        assert_not_serialized(&value);
        assert_redacted(
            &value,
            &[MARKER],
            &["RetrievalResult", "snippet_len", "score"],
        );
    }

    #[test]
    fn retrieval_response_debug_withholds_excluded_reasons() {
        let value = RetrievalResponse {
            schema: "ascension.context-memory.retrieval.v1".to_owned(),
            query_id: "query-1".to_owned(),
            scope: scope(),
            branch_id: "branch-a".to_owned(),
            query_sha256: sha256_hex(b"query"),
            cutoff: 10,
            corpus_generation: 1,
            projection_generation: 1,
            revocation_epoch: 0,
            ranker_version: "lexical-v1".to_owned(),
            results: Vec::new(),
            coverage: RetrievalCoverage::CompleteWithinScope,
            inference_calls: 0,
            excluded: vec![exclusion()],
        };
        let leaked = byte_array(MARKER);
        assert_not_serialized(&value);
        assert_redacted(
            &value,
            &[MARKER, leaked.as_str(), "reason:"],
            &["RetrievalResponse", "exclusion_count", "query_id"],
        );
    }

    #[test]
    fn memory_proposal_debug_withholds_content() {
        let value = MemoryProposal {
            schema: "ascension.context-memory.proposal.v1".to_owned(),
            proposal_id: "proposal-1".to_owned(),
            version: 1,
            scope: scope(),
            branch_id: "branch-a".to_owned(),
            kind: ProposalKind::Extractive,
            sources: Vec::new(),
            cutoff: 10,
            corpus_generation: 1,
            claims: Vec::new(),
            omissions: Vec::new(),
            contradictions: Vec::new(),
            lineage_depth: 0,
            status: ProposalStatus::Generated,
            content_ref: "content-ref".to_owned(),
            sha256: sha256_hex(MARKER.as_bytes()),
            byte_length: MARKER.len(),
            source_reconstruction: SourceReconstruction::Available,
            created_at: "2026-09-10T10:00:00Z".to_owned(),
            expires_at: "2026-09-11T12:00:00Z".to_owned(),
            applied: false,
            content: MARKER.as_bytes().to_vec(),
        };
        let leaked = byte_array(MARKER);
        assert_not_serialized(&value);
        assert_redacted(
            &value,
            &[MARKER, leaked.as_str()],
            &["MemoryProposal", "content_len", "proposal_id"],
        );
    }
}
