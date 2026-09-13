// SPDX-License-Identifier: MIT

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryCapabilities {
    pub schema: String,
    pub product_phase: u8,
    pub scope: MemoryScope,
    pub enabled: bool,
    pub local_lexical_retrieval: String,
    pub extractive_compaction: String,
    pub abstractive_adapter: String,
    pub abstractive_live_verified: bool,
    pub per_decision_policy: String,
    pub phase2_approval_required: bool,
    pub persistent_provider_sessions: bool,
    pub provider_side_compaction: bool,
    pub semantic_vector_retrieval: String,
    pub hidden_reasoning_access: bool,
    pub direct_game_dispatch: bool,
    /// Owner-enforced limits, separate from portable JSON Schema ceilings.
    pub effective_limits: EffectiveMemoryLimits,
    /// Provenance and integrity metadata; the digest covers this payload with its field cleared.
    pub binding: MemoryCapabilityBinding,
    pub supported_operations: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryCapabilityBinding {
    pub owner: String,
    pub owner_revision: String,
    pub policy_schema_sha256: String,
    /// Explicit sentinel: context memory is not model-owned.
    pub model_revision: String,
    pub adapter_revision: String,
    pub adapter_revision_sha256: String,
    pub descriptor_sha256: String,
}

/// Owner-enforced policy limits bound to a capability response.
///
/// A policy can be valid against the public schema while exceeding one of these values. Callers
/// must use this descriptor for admission and preserve an over-limit policy for inspection rather
/// than clamping it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveMemoryLimits {
    pub policy_schema: String,
    pub max_candidates: usize,
    pub max_results: usize,
    pub max_selected: usize,
    pub optional_byte_budget: usize,
    pub max_entries_per_run: usize,
    pub max_corpus_bytes: usize,
    pub max_source_bytes: usize,
    pub max_sources_per_job: usize,
    pub max_job_input_bytes: usize,
    pub max_summary_output_bytes: usize,
    pub max_query_bytes: usize,
    pub max_lineage_depth: u8,
    pub max_global_memory_bytes: usize,
    pub max_global_memory_jobs: usize,
    pub max_retention_resources: usize,
    pub max_retention_bytes: usize,
    pub max_cache_entries: usize,
    pub max_review_records: usize,
    pub max_memory_bindings: usize,
    pub max_usage_attempts: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryLimitViolation {
    pub limit: String,
    pub requested: usize,
    pub effective: usize,
}

impl EffectiveMemoryLimits {
    fn validate(&self) -> Result<(), MemoryError> {
        if self.policy_schema != MEMORY_POLICY_SCHEMA
            || self.max_candidates == 0
            || self.max_candidates > MAX_CANDIDATES
            || self.max_results == 0
            || self.max_results > MAX_RESULTS
            || self.max_selected == 0
            || self.max_selected > MAX_SELECTED
            || self.optional_byte_budget == 0
            || self.optional_byte_budget > MAX_OPTIONAL_BYTES
            || self.max_entries_per_run == 0
            || self.max_entries_per_run > MAX_ENTRIES_PER_RUN
            || self.max_corpus_bytes == 0
            || self.max_corpus_bytes > MAX_CORPUS_BYTES
            || self.max_source_bytes == 0
            || self.max_source_bytes > MAX_SOURCE_BYTES
            || self.max_sources_per_job == 0
            || self.max_sources_per_job > MAX_SOURCES_PER_JOB
            || self.max_job_input_bytes == 0
            || self.max_job_input_bytes > MAX_JOB_INPUT_BYTES
            || self.max_summary_output_bytes == 0
            || self.max_summary_output_bytes > MAX_SUMMARY_OUTPUT_BYTES
            || self.max_query_bytes == 0
            || self.max_query_bytes > MAX_QUERY_BYTES
            || self.max_lineage_depth == 0
            || self.max_lineage_depth > MAX_LINEAGE_DEPTH
            || self.max_global_memory_bytes == 0
            || self.max_global_memory_bytes > MAX_GLOBAL_MEMORY_BYTES
            || self.max_global_memory_jobs == 0
            || self.max_global_memory_jobs > MAX_GLOBAL_MEMORY_JOBS
            || self.max_retention_resources == 0
            || self.max_retention_resources > MAX_RETENTION_RESOURCES
            || self.max_retention_bytes == 0
            || self.max_retention_bytes > MAX_RETENTION_BYTES
            || self.max_cache_entries == 0
            || self.max_cache_entries > MAX_CACHE_ENTRIES
            || self.max_review_records == 0
            || self.max_review_records > MAX_REVIEW_RECORDS
            || self.max_memory_bindings == 0
            || self.max_memory_bindings > MAX_MEMORY_BINDINGS
            || self.max_usage_attempts == 0
            || self.max_usage_attempts > MAX_USAGE_ATTEMPTS
        {
            return Err(MemoryError::InvalidCapabilities);
        }
        Ok(())
    }
}

impl MemoryCapabilityBinding {
    fn validate(&self) -> Result<(), MemoryError> {
        if self.owner != MEMORY_CAPABILITIES_OWNER
            || self.owner_revision != MEMORY_CAPABILITIES_REVISION
            || self.policy_schema_sha256 != memory_policy_schema_sha256()
            || self.model_revision != "not-applicable"
            || self.adapter_revision != MEMORY_CAPABILITIES_REVISION
            || self.adapter_revision_sha256 != sha256_hex(self.adapter_revision.as_bytes())
            || !valid_digest(&self.descriptor_sha256)
        {
            return Err(MemoryError::InvalidCapabilities);
        }
        Ok(())
    }
}

impl MemoryCorpus {
    pub fn capabilities(&self) -> MemoryCapabilities {
        let mut capabilities = MemoryCapabilities {
            schema: MEMORY_CAPABILITIES_SCHEMA.to_owned(),
            product_phase: 3,
            scope: self.scope.clone(),
            enabled: self.enabled,
            local_lexical_retrieval: "supported".to_owned(),
            extractive_compaction: "supported".to_owned(),
            abstractive_adapter: "supported".to_owned(),
            abstractive_live_verified: false,
            per_decision_policy: "supported".to_owned(),
            phase2_approval_required: true,
            persistent_provider_sessions: false,
            provider_side_compaction: false,
            semantic_vector_retrieval: "unsupported".to_owned(),
            hidden_reasoning_access: false,
            direct_game_dispatch: false,
            effective_limits: EffectiveMemoryLimits {
                policy_schema: MEMORY_POLICY_SCHEMA.to_owned(),
                max_candidates: MAX_CANDIDATES,
                max_results: MAX_RESULTS,
                max_selected: MAX_SELECTED,
                optional_byte_budget: MAX_OPTIONAL_BYTES,
                max_entries_per_run: self.max_entries,
                max_corpus_bytes: self.max_bytes,
                max_source_bytes: MAX_SOURCE_BYTES,
                max_sources_per_job: MAX_SOURCES_PER_JOB,
                max_job_input_bytes: MAX_JOB_INPUT_BYTES,
                max_summary_output_bytes: MAX_SUMMARY_OUTPUT_BYTES,
                max_query_bytes: MAX_QUERY_BYTES,
                max_lineage_depth: MAX_LINEAGE_DEPTH,
                max_global_memory_bytes: MAX_GLOBAL_MEMORY_BYTES,
                max_global_memory_jobs: MAX_GLOBAL_MEMORY_JOBS,
                max_retention_resources: MAX_RETENTION_RESOURCES,
                max_retention_bytes: MAX_RETENTION_BYTES,
                max_cache_entries: MAX_CACHE_ENTRIES,
                max_review_records: MAX_REVIEW_RECORDS,
                max_memory_bindings: MAX_MEMORY_BINDINGS,
                max_usage_attempts: MAX_USAGE_ATTEMPTS,
            },
            binding: MemoryCapabilityBinding {
                owner: MEMORY_CAPABILITIES_OWNER.to_owned(),
                owner_revision: MEMORY_CAPABILITIES_REVISION.to_owned(),
                policy_schema_sha256: memory_policy_schema_sha256(),
                model_revision: "not-applicable".to_owned(),
                adapter_revision: MEMORY_CAPABILITIES_REVISION.to_owned(),
                adapter_revision_sha256: sha256_hex(MEMORY_CAPABILITIES_REVISION.as_bytes()),
                descriptor_sha256: String::new(),
            },
            supported_operations: if self.enabled {
                vec![
                    "search", "extract", "generate", "review", "select", "policy", "adopt",
                    "revoke", "evaluate",
                ]
                .into_iter()
                .map(str::to_owned)
                .collect()
            } else {
                Vec::new()
            },
        };
        capabilities.binding.descriptor_sha256 = capabilities.descriptor_digest();
        capabilities
    }
}

impl MemoryCapabilities {
    /// Validate an owner-produced descriptor before using any advertised limit.
    pub fn validate(&self) -> Result<(), MemoryError> {
        if self.schema != MEMORY_CAPABILITIES_SCHEMA
            || self.product_phase != 3
            || !self.scope.valid()
            || !self.phase2_approval_required
            || self.persistent_provider_sessions
            || self.provider_side_compaction
            || self.semantic_vector_retrieval != "unsupported"
            || self.hidden_reasoning_access
            || self.direct_game_dispatch
            || self.supported_operations.len() > 9
            || self
                .supported_operations
                .iter()
                .any(|operation| !valid_id(operation))
            || self
                .supported_operations
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                != self.supported_operations.len()
            || (!self.enabled && !self.supported_operations.is_empty())
        {
            return Err(MemoryError::InvalidCapabilities);
        }
        self.effective_limits.validate()?;
        self.binding.validate()?;
        if self.binding.descriptor_sha256 != self.descriptor_digest() {
            return Err(MemoryError::InvalidCapabilities);
        }
        Ok(())
    }

    /// Validate the descriptor against revisions supplied by a trusted consumer configuration.
    /// `validate` checks the harness defaults; this method is for forwarded descriptors selected
    /// by another owner/profile and must not be called with values copied from the descriptor.
    pub fn validate_against_trusted(
        &self,
        owner_revision: &str,
        adapter_revision: &str,
        policy_schema_sha256: &str,
    ) -> Result<(), MemoryError> {
        self.validate()?;
        if self.binding.owner_revision != owner_revision
            || self.binding.adapter_revision != adapter_revision
            || self.binding.policy_schema_sha256 != policy_schema_sha256
        {
            return Err(MemoryError::InvalidCapabilities);
        }
        Ok(())
    }

    #[must_use]
    pub fn descriptor_digest(&self) -> String {
        let mut unsigned = self.clone();
        unsigned.binding.descriptor_sha256.clear();
        sha256_hex(serde_json::to_vec(&unsigned).unwrap_or_default())
    }
}

#[must_use]
pub fn memory_policy_schema_sha256() -> String {
    sha256_hex(include_bytes!("../../../../contracts/context-memory/policy.schema.json"))
}
