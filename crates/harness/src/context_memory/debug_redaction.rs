// SPDX-License-Identifier: MIT

// Several records in this module derive `Debug` over `#[serde(skip)]` fields. A serde skip governs
// serialization only, so the derived impls printed those fields - entry and backup content,
// attachment bytes, prepared rendered bytes, retrieval snippets, exclusion reasons - through `{:?}`
// and `{:#?}`. Each impl below is allowlisted: it reports non-sensitive identity and shape metadata
// only, rendering byte fields as lengths and collections as counts, and every impl terminates with
// `finish_non_exhaustive()` so a future sensitive field cannot silently re-enter the format path.
// The impls live here rather than beside their types so the per-file size budget is preserved; the
// includes share one module namespace, so trait impls may be written in any participating file.

impl std::fmt::Debug for MemoryEntry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MemoryEntry")
            .field("schema", &self.schema)
            .field("entry_id", &self.entry_id)
            .field("version", &self.version)
            .field("scope", &self.scope)
            .field("branch_id", &self.branch_id)
            .field("source_record_id", &self.source_record_id)
            .field("kind", &self.kind)
            .field("evidence", &self.evidence)
            .field("authority", &self.authority)
            .field("observed_seq", &self.observed_seq)
            .field("admitted_seq", &self.admitted_seq)
            .field("corpus_generation", &self.corpus_generation)
            .field("content_ref", &self.content_ref)
            .field("sha256", &self.sha256)
            .field("byte_length", &self.byte_length)
            .field("parent_count", &self.parents.len())
            .field("lineage_depth", &self.lineage_depth)
            .field("status", &self.status)
            .field("created_at", &self.created_at)
            .field("expires_at", &self.expires_at)
            .field("game_profile", &self.game_profile)
            .field("content_len", &self.content.len())
            .field("protected", &self.protected)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for MemoryBackupEntry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MemoryBackupEntry")
            .field("metadata", &self.metadata)
            .field("content_len", &self.content.len())
            .field("protected", &self.protected)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for MapAttachment {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MapAttachment")
            .field("kind", &self.kind)
            .field("artifact_id", &self.artifact_id)
            .field("generation", &self.generation)
            .field("sha256", &self.sha256)
            .field("byte_length", &self.byte_length)
            .field("mime", &self.mime)
            .field("bytes_len", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for SelectionManifest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SelectionManifest")
            .field("schema", &self.schema)
            .field("selection_id", &self.selection_id)
            .field("scope", &self.scope)
            .field("branch_id", &self.branch_id)
            .field("policy_id", &self.policy_id)
            .field("policy_version", &self.policy_version)
            .field("cutoff", &self.cutoff)
            .field("corpus_generation", &self.corpus_generation)
            .field("revocation_epoch", &self.revocation_epoch)
            .field("selected_source_count", &self.selected_sources.len())
            .field("pinned_entry_count", &self.pinned_entry_ids.len())
            .field(
                "protected_manifest_sha256",
                &self.protected_manifest_sha256,
            )
            .field("optional_byte_budget", &self.optional_byte_budget)
            .field("optional_rendered_bytes", &self.optional_rendered_bytes)
            .field("whole_rendered_bytes", &self.whole_rendered_bytes)
            .field("prepared_manifest_sha256", &self.prepared_manifest_sha256)
            .field("whole_tokens", &self.whole_tokens)
            .field("token_measurement", &self.token_measurement)
            .field("budget_status", &self.budget_status)
            .field("rendered_content_ref", &self.rendered_content_ref)
            .field("expires_at", &self.expires_at)
            .field("phase2_revision_id", &self.phase2_revision_id)
            .field("effect_class", &self.effect_class)
            .field(
                "phase2_prepared_manifest_sha256",
                &self.phase2_prepared_manifest_sha256,
            )
            .field("rendered_bytes_len", &self.rendered_bytes.len())
            .field("exclusion_count", &self.exclusions.len())
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for RetrievalResult {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RetrievalResult")
            .field("source", &self.source)
            .field("score", &self.score)
            .field("reason_count", &self.reasons.len())
            .field(
                "snippet_len",
                &self.snippet.as_ref().map_or(0, String::len),
            )
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for RetrievalResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RetrievalResponse")
            .field("schema", &self.schema)
            .field("query_id", &self.query_id)
            .field("scope", &self.scope)
            .field("branch_id", &self.branch_id)
            .field("query_sha256", &self.query_sha256)
            .field("cutoff", &self.cutoff)
            .field("corpus_generation", &self.corpus_generation)
            .field("projection_generation", &self.projection_generation)
            .field("revocation_epoch", &self.revocation_epoch)
            .field("ranker_version", &self.ranker_version)
            .field("result_count", &self.results.len())
            .field("coverage", &self.coverage)
            .field("inference_calls", &self.inference_calls)
            .field("exclusion_count", &self.excluded.len())
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for MemoryProposal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MemoryProposal")
            .field("schema", &self.schema)
            .field("proposal_id", &self.proposal_id)
            .field("version", &self.version)
            .field("scope", &self.scope)
            .field("branch_id", &self.branch_id)
            .field("kind", &self.kind)
            .field("source_count", &self.sources.len())
            .field("cutoff", &self.cutoff)
            .field("corpus_generation", &self.corpus_generation)
            .field("claim_count", &self.claims.len())
            .field("omission_count", &self.omissions.len())
            .field("contradiction_count", &self.contradictions.len())
            .field("lineage_depth", &self.lineage_depth)
            .field("status", &self.status)
            .field("content_ref", &self.content_ref)
            .field("sha256", &self.sha256)
            .field("byte_length", &self.byte_length)
            .field("source_reconstruction", &self.source_reconstruction)
            .field("created_at", &self.created_at)
            .field("expires_at", &self.expires_at)
            .field("applied", &self.applied)
            .field("content_len", &self.content.len())
            .finish_non_exhaustive()
    }
}
