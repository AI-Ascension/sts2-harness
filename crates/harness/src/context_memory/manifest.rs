// SPDX-License-Identifier: MIT

#[must_use]
pub fn source_manifest_digest(sources: &[MemoryRef]) -> String {
    #[derive(Serialize)]
    struct CanonicalRef<'a> {
        entry_id: &'a str,
        sha256: &'a str,
        version: u64,
    }

    let canonical = sources
        .iter()
        .map(|reference| CanonicalRef {
            entry_id: &reference.entry_id,
            sha256: &reference.sha256,
            version: reference.version,
        })
        .collect::<Vec<_>>();
    let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
    sha256_hex(bytes)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyMode {
    ManualSnapshot,
    BoundedPerDecision,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyStatus {
    Draft,
    Approved,
    Revoked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionFallback {
    Block,
    ProtectedOnly,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryPolicy {
    pub schema: String,
    pub policy_id: String,
    pub version: u64,
    pub scope: MemoryScope,
    pub mode: PolicyMode,
    pub status: PolicyStatus,
    pub phase2_revision_id: Option<String>,
    pub corpus_generation: u64,
    pub rolling_same_episode_sources: bool,
    pub cross_scope: bool,
    pub approved_summary_catalog: Vec<MemoryRef>,
    pub ranker_version: String,
    pub query_derivation_version: String,
    pub max_candidates: usize,
    pub max_results: usize,
    pub max_selected: usize,
    pub optional_byte_budget: usize,
    pub fallback: SelectionFallback,
    pub automatic_summary_activation: bool,
    pub generate_during_selection: bool,
    pub authorization_policy_version: String,
}

impl MemoryPolicy {
    pub fn validate(&self, corpus: &MemoryCorpus) -> Result<(), MemoryError> {
        if self.schema != MEMORY_POLICY_SCHEMA
            || !valid_id(&self.policy_id)
            || self.version == 0
            || self.scope != *corpus.scope()
            || self.phase2_revision_id.is_none()
            || self.corpus_generation == 0
            || self.corpus_generation > corpus.generation()
            || self.cross_scope
            || self.status != PolicyStatus::Approved
            || self.max_candidates == 0
            || self.max_candidates > MAX_CANDIDATES
            || self.max_results == 0
            || self.max_results > MAX_RESULTS
            || self.max_selected == 0
            || self.max_selected > MAX_SELECTED
            || self.optional_byte_budget == 0
            || self.optional_byte_budget > MAX_OPTIONAL_BYTES
            || self.automatic_summary_activation
            || self.generate_during_selection
            || !valid_id(&self.ranker_version)
            || !valid_id(&self.query_derivation_version)
            || !valid_id(&self.authorization_policy_version)
            || (self.mode == PolicyMode::ManualSnapshot && self.rolling_same_episode_sources)
            || self.status == PolicyStatus::Revoked
            || self.max_results > self.max_candidates
        {
            return Err(MemoryError::InvalidQuery);
        }
        if self.approved_summary_catalog.len() > 32
            || self
                .approved_summary_catalog
                .iter()
                .any(|reference| !reference.valid())
            || self.approved_summary_catalog.iter().collect::<BTreeSet<_>>().len()
                != self.approved_summary_catalog.len()
        {
            return Err(MemoryError::InvalidQuery);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectionRequest {
    pub selection_id: String,
    pub policy: MemoryPolicy,
    pub branch_id: String,
    pub cutoff: u64,
    pub corpus_generation: u64,
    pub mandatory_bytes: Vec<u8>,
    pub mandatory_manifest_sha256: String,
    pub optional_sources: Vec<MemoryRef>,
    pub pinned_entry_ids: Vec<String>,
    pub prepared_content_ref: String,
    pub expires_at: String,
    pub phase2_prepared_manifest_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectionManifest {
    pub schema: String,
    pub selection_id: String,
    pub scope: MemoryScope,
    pub branch_id: String,
    pub policy_id: String,
    pub policy_version: u64,
    pub cutoff: u64,
    pub corpus_generation: u64,
    pub revocation_epoch: u64,
    pub selected_sources: Vec<MemoryRef>,
    pub pinned_entry_ids: Vec<String>,
    pub protected_manifest_sha256: String,
    pub optional_byte_budget: usize,
    pub optional_rendered_bytes: usize,
    pub whole_rendered_bytes: usize,
    pub prepared_manifest_sha256: String,
    pub whole_tokens: Option<u64>,
    pub token_measurement: String,
    pub budget_status: String,
    pub rendered_content_ref: String,
    pub expires_at: String,
    pub phase2_revision_id: String,
    pub effect_class: String,
    #[serde(skip)]
    pub phase2_prepared_manifest_sha256: String,
    #[serde(skip)]
    pub rendered_bytes: Vec<u8>,
    #[serde(skip)]
    pub exclusions: Vec<ExclusionReason>,
}
