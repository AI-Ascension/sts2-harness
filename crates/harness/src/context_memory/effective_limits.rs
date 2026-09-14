// SPDX-License-Identifier: MIT

use crate::effective_limits::{
    EFFECTIVE_LIMIT_RECORD_SCHEMA, EffectiveLimitRecord, LimitRow, UnavailableReason,
};

const MEMORY_POLICY_LIMIT_VALIDATOR: &str =
    "MemoryPolicy::validate_schema+validate_against_capabilities";
const MEMORY_CORPUS_LIMIT_VALIDATOR: &str = "MemoryCorpus::with_limits";

impl MemoryCapabilities {
    /// Publish the schema-valid versus executable classification for every advertised value.
    ///
    /// A policy value can be valid against the portable schema and still not execute on the
    /// selected profile. Consumers must read the executable ceiling from this record instead of
    /// copying a schema maximum into a form, and must treat a missing field as unavailable.
    #[must_use]
    pub fn effective_limit_record(&self) -> EffectiveLimitRecord {
        let limits = &self.effective_limits;
        let rows = vec![
            LimitRow::policy(
                "max_candidates",
                MAX_CANDIDATES as u64,
                MAX_CANDIDATES as u64,
                limits.max_candidates as u64,
                MEMORY_POLICY_LIMIT_VALIDATOR,
            ),
            LimitRow::policy(
                "max_results",
                MAX_RESULTS as u64,
                MAX_RESULTS as u64,
                limits.max_results as u64,
                MEMORY_POLICY_LIMIT_VALIDATOR,
            ),
            LimitRow::policy(
                "max_selected",
                MAX_SELECTED as u64,
                MAX_SELECTED as u64,
                limits.max_selected as u64,
                MEMORY_POLICY_LIMIT_VALIDATOR,
            ),
            LimitRow::policy(
                "optional_byte_budget",
                MEMORY_POLICY_SCHEMA_MAX_OPTIONAL_BYTES as u64,
                MAX_OPTIONAL_BYTES as u64,
                limits.optional_byte_budget as u64,
                MEMORY_POLICY_LIMIT_VALIDATOR,
            ),
            LimitRow::profile_selected(
                "max_entries_per_run",
                MAX_ENTRIES_PER_RUN as u64,
                limits.max_entries_per_run as u64,
                MEMORY_CORPUS_LIMIT_VALIDATOR,
            ),
            LimitRow::profile_selected(
                "max_corpus_bytes",
                MAX_CORPUS_BYTES as u64,
                limits.max_corpus_bytes as u64,
                MEMORY_CORPUS_LIMIT_VALIDATOR,
            ),
            LimitRow::runtime_guard(
                "max_source_bytes",
                limits.max_source_bytes as u64,
                "MemoryCorpus admission",
            ),
            LimitRow::runtime_guard(
                "max_sources_per_job",
                limits.max_sources_per_job as u64,
                "SummaryJob admission",
            ),
            LimitRow::runtime_guard(
                "max_job_input_bytes",
                limits.max_job_input_bytes as u64,
                "SummaryJob budget admission",
            ),
            LimitRow::runtime_guard(
                "max_summary_output_bytes",
                limits.max_summary_output_bytes as u64,
                "SummaryJob output admission",
            ),
            LimitRow::runtime_guard(
                "max_query_bytes",
                limits.max_query_bytes as u64,
                "MemoryQuery validation",
            ),
            LimitRow::runtime_guard(
                "max_lineage_depth",
                u64::from(limits.max_lineage_depth),
                "MemoryEntry lineage admission",
            ),
            LimitRow::runtime_guard(
                "max_global_memory_bytes",
                limits.max_global_memory_bytes as u64,
                "MemoryBudgetLedger",
            ),
            LimitRow::runtime_guard(
                "max_global_memory_jobs",
                limits.max_global_memory_jobs as u64,
                "MemoryBudgetLedger",
            ),
            LimitRow::runtime_guard(
                "max_retention_resources",
                limits.max_retention_resources as u64,
                "RetentionInventory",
            ),
            LimitRow::runtime_guard(
                "max_retention_bytes",
                limits.max_retention_bytes as u64,
                "RetentionInventory",
            ),
            LimitRow::runtime_guard(
                "max_cache_entries",
                limits.max_cache_entries as u64,
                "RetrievalCache",
            ),
            LimitRow::runtime_guard(
                "max_review_records",
                limits.max_review_records as u64,
                "ImmutableReviewLedger",
            ),
            LimitRow::runtime_guard(
                "max_memory_bindings",
                limits.max_memory_bindings as u64,
                "AtomicBindingStore",
            ),
            LimitRow::runtime_guard(
                "max_usage_attempts",
                limits.max_usage_attempts as u64,
                "UsageLedger",
            ),
        ];
        EffectiveLimitRecord {
            schema: EFFECTIVE_LIMIT_RECORD_SCHEMA.to_owned(),
            surface: "context-memory".to_owned(),
            owner: self.binding.owner.clone(),
            owner_revision: self.binding.owner_revision.clone(),
            capability_schema: self.schema.clone(),
            capability_descriptor_sha256: self.binding.descriptor_sha256.clone(),
            enabled: self.enabled,
            rows,
        }
    }

    /// Selected-profile admission for a policy value. Schema validity is checked separately by
    /// [`MemoryPolicy::validate_schema`]; this answers only whether the profile can execute it.
    ///
    /// # Errors
    ///
    /// Returns [`UnavailableReason`] when memory is disabled, the field is not advertised, or the
    /// value exceeds the executable ceiling.
    pub fn admit_policy_value(
        &self,
        field: &str,
        requested: u64,
    ) -> Result<(), UnavailableReason> {
        self.effective_limit_record().admit(field, requested)
    }
}
