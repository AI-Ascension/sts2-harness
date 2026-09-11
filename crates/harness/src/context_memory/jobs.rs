// SPDX-License-Identifier: MIT

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Executing,
    Succeeded,
    Rejected,
    Cancelled,
    Expired,
    Blocked,
    OutcomeUnknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderWriteState {
    NotStarted,
    IntentPersisted,
    PossiblyWritten,
    Completed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SummaryJob {
    pub schema: String,
    pub job_id: String,
    pub scope: MemoryScope,
    pub branch_id: String,
    pub sources: Vec<MemoryRef>,
    pub cutoff: u64,
    pub corpus_generation: u64,
    pub source_manifest_sha256: String,
    pub generator_profile: String,
    pub generator_prompt_sha256: String,
    pub output_schema_sha256: String,
    pub idempotency_key: String,
    pub command_window_id: String,
    pub state: JobState,
    pub attempt_id: Option<String>,
    pub provider_write_state: ProviderWriteState,
    pub input_bytes: usize,
    pub max_output_bytes: usize,
    pub review_required: bool,
    pub auto_apply: bool,
    pub deadline_at: String,
    pub effect_class: String,
}

impl SummaryJob {
    pub fn validate(&self) -> Result<(), MemoryError> {
        if self.schema != MEMORY_JOB_SCHEMA
            || !valid_id(&self.job_id)
            || !self.scope.valid()
            || !valid_id(&self.branch_id)
            || self.sources.is_empty()
            || self.sources.len() > MAX_SOURCES_PER_JOB
            || self.sources.iter().any(|reference| !reference.valid())
            || self.sources.iter().collect::<BTreeSet<_>>().len() != self.sources.len()
            || self.cutoff > 9_007_199_254_740_991
            || self.corpus_generation == 0
            || self.corpus_generation > 9_007_199_254_740_991
            || !valid_digest(&self.source_manifest_sha256)
            || !valid_id(&self.generator_profile)
            || !valid_digest(&self.generator_prompt_sha256)
            || !valid_digest(&self.output_schema_sha256)
            || !valid_id(&self.idempotency_key)
            || !valid_id(&self.command_window_id)
            || self.input_bytes > MAX_JOB_INPUT_BYTES
            || self.max_output_bytes == 0
            || self.max_output_bytes > MAX_SUMMARY_OUTPUT_BYTES
            || !self.review_required
            || self.auto_apply
            || !valid_timestamp(&self.deadline_at)
            || self.effect_class != "authorized_summary_generation_only"
        {
            return Err(MemoryError::InvalidProposal);
        }
        if self.state == JobState::Queued
            && (self.attempt_id.is_some()
                || self.provider_write_state != ProviderWriteState::NotStarted)
        {
            return Err(MemoryError::InvalidProposal);
        }
        if self.state == JobState::OutcomeUnknown
            && (self.attempt_id.is_none()
                || self.provider_write_state != ProviderWriteState::PossiblyWritten)
        {
            return Err(MemoryError::InvalidProposal);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SummaryGenerationRequest {
    pub job_id: String,
    pub generator_profile: String,
    pub prompt_sha256: String,
    pub output_schema_sha256: String,
    pub source_bytes: Vec<(MemoryRef, Vec<u8>)>,
    pub max_output_bytes: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SummaryGeneration {
    pub output: Vec<u8>,
    pub output_sha256: String,
}

pub trait SummaryProvider {
    fn generate(
        &mut self,
        request: SummaryGenerationRequest,
    ) -> Result<SummaryGeneration, MemoryError>;
}

/// A deterministic fake peer used at the external provider boundary.  It records only bounded
/// source identities/bytes and cannot access the game, management, shell, or arbitrary network.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeSummaryPeer {
    output: Vec<u8>,
    requests: Vec<SummaryGenerationRequest>,
    max_requests: usize,
}

impl FakeSummaryPeer {
    pub fn new(output: Vec<u8>) -> Result<Self, MemoryError> {
        if output.is_empty() || output.len() > MAX_SUMMARY_OUTPUT_BYTES {
            return Err(MemoryError::Capacity);
        }
        Ok(Self {
            output,
            requests: Vec::new(),
            max_requests: 2,
        })
    }

    pub fn requests(&self) -> &[SummaryGenerationRequest] {
        &self.requests
    }
}

impl SummaryProvider for FakeSummaryPeer {
    fn generate(
        &mut self,
        request: SummaryGenerationRequest,
    ) -> Result<SummaryGeneration, MemoryError> {
        if self.requests.len() >= self.max_requests
            || request.source_bytes.len() > MAX_SOURCES_PER_JOB
            || request.max_output_bytes < self.output.len()
        {
            return Err(MemoryError::Capacity);
        }
        let bytes: usize = request
            .source_bytes
            .iter()
            .map(|(_, bytes)| bytes.len())
            .sum();
        if bytes > MAX_JOB_INPUT_BYTES {
            return Err(MemoryError::Capacity);
        }
        self.requests.push(request);
        Ok(SummaryGeneration {
            output: self.output.clone(),
            output_sha256: sha256_hex(&self.output),
        })
    }
}
