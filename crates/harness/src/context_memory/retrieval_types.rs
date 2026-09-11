// SPDX-License-Identifier: MIT

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryQuery {
    pub schema: String,
    /// Query identity is a local response correlation and is intentionally not part of query.v1.
    #[serde(skip)]
    pub query_id: String,
    pub scope: MemoryScope,
    pub branch_id: String,
    pub query: String,
    pub cutoff: u64,
    pub corpus_generation: u64,
    pub ranker_version: String,
    pub limit: usize,
    pub max_candidates: usize,
    pub effect_class: String,
}

impl MemoryQuery {
    pub fn validate(&self) -> Result<(), MemoryError> {
        if self.schema != MEMORY_QUERY_SCHEMA
            || !self.scope.valid()
            || !valid_id(&self.branch_id)
            || self.query.is_empty()
            || self.query.len() > MAX_QUERY_BYTES
            || self.cutoff > 9_007_199_254_740_991
            || self.corpus_generation == 0
            || self.corpus_generation > 9_007_199_254_740_991
            || !valid_id(&self.ranker_version)
            || self.limit == 0
            || self.limit > MAX_RESULTS
            || self.max_candidates == 0
            || self.max_candidates > MAX_CANDIDATES
            || self.effect_class != "local_read_no_inference"
        {
            return Err(MemoryError::InvalidQuery);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetrievalResult {
    pub source: MemoryRef,
    pub score: i64,
    pub reasons: Vec<String>,
    #[serde(skip)]
    pub snippet: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalCoverage {
    CompleteWithinScope,
    CandidateLimited,
    TimeLimited,
    ProjectionUnavailable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExclusionReason {
    pub entry_id: String,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetrievalResponse {
    pub schema: String,
    pub query_id: String,
    pub scope: MemoryScope,
    pub branch_id: String,
    pub query_sha256: String,
    pub cutoff: u64,
    pub corpus_generation: u64,
    pub projection_generation: u64,
    pub revocation_epoch: u64,
    pub ranker_version: String,
    pub results: Vec<RetrievalResult>,
    pub coverage: RetrievalCoverage,
    pub inference_calls: u32,
    #[serde(skip)]
    pub excluded: Vec<ExclusionReason>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimSupport {
    Extractive,
    Reported,
    Inferred,
    Unknown,
    Contradictory,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Applicability {
    Historical,
    Conditional,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Citation {
    pub source: MemoryRef,
    pub start_byte: usize,
    pub end_byte: usize,
    pub quote_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryClaim {
    pub claim_id: String,
    pub text: String,
    pub support: ClaimSupport,
    pub citations: Vec<Citation>,
    pub uncertainty: String,
    pub applicability: Applicability,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalKind {
    Extractive,
    Abstractive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Generated,
    MachineChecked,
    ReviewRequired,
    Rejected,
    Stale,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceReconstruction {
    Available,
    Partial,
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryProposal {
    pub schema: String,
    pub proposal_id: String,
    pub version: u64,
    pub scope: MemoryScope,
    pub branch_id: String,
    pub kind: ProposalKind,
    pub sources: Vec<MemoryRef>,
    pub cutoff: u64,
    pub corpus_generation: u64,
    pub claims: Vec<MemoryClaim>,
    pub omissions: Vec<String>,
    pub contradictions: Vec<String>,
    pub lineage_depth: u8,
    pub status: ProposalStatus,
    pub content_ref: String,
    pub sha256: String,
    pub byte_length: usize,
    pub source_reconstruction: SourceReconstruction,
    pub created_at: String,
    pub expires_at: String,
    pub applied: bool,
    #[serde(skip)]
    pub content: Vec<u8>,
}

impl MemoryProposal {
    pub fn validate_against(&self, corpus: &MemoryCorpus, now: &str) -> Result<(), MemoryError> {
        if self.schema != MEMORY_PROPOSAL_SCHEMA
            || !valid_id(&self.proposal_id)
            || self.version == 0
            || self.version > 9_007_199_254_740_991
            || self.scope != *corpus.scope()
            || !valid_id(&self.branch_id)
            || self.sources.is_empty()
            || self.sources.len() > MAX_SOURCES_PER_JOB
            || self.claims.len() > 32
            || self.omissions.len() > 32
            || self.contradictions.len() > 16
            || self.lineage_depth == 0
            || self.lineage_depth > MAX_LINEAGE_DEPTH
            || self.cutoff > 9_007_199_254_740_991
            || self.corpus_generation == 0
            || self.corpus_generation > 9_007_199_254_740_991
            || !valid_id(&self.content_ref)
            || !valid_timestamp(&self.created_at)
            || !valid_timestamp(&self.expires_at)
            || self.created_at.as_str() > now
            || self.expires_at.as_str() <= self.created_at.as_str()
            || self.applied
            || self.byte_length > MAX_SUMMARY_OUTPUT_BYTES
            || self.content.len() != self.byte_length
            || sha256_hex(&self.content) != self.sha256
            || !valid_digest(&self.sha256)
            || !valid_timestamp(now)
            || self.expires_at.as_str() <= now
        {
            return Err(MemoryError::InvalidProposal);
        }
        let source_set: BTreeSet<MemoryRef> = self.sources.iter().cloned().collect();
        if source_set.len() != self.sources.len() {
            return Err(MemoryError::InvalidProposal);
        }
        if self.claims.iter().map(|claim| &claim.claim_id).collect::<BTreeSet<_>>().len()
            != self.claims.len()
            || self.omissions.iter().any(|omission| omission.is_empty() || omission.len() > 4096)
            || self
                .contradictions
                .iter()
                .any(|contradiction| contradiction.is_empty() || contradiction.len() > 4096)
        {
            return Err(MemoryError::InvalidProposal);
        }
        let mut shortest_expiry: Option<&str> = None;
        let mut deepest_parent = 0_u8;
        for source in &self.sources {
            let entry = corpus.entry(source).ok_or(MemoryError::MissingParent)?;
            corpus.eligible_entry(
                entry,
                &self.branch_id,
                self.cutoff,
                self.corpus_generation,
                now,
                false,
            )?;
            shortest_expiry = Some(
                shortest_expiry
                    .map_or(entry.expires_at.as_str(), |current| {
                        current.min(entry.expires_at.as_str())
                    }),
            );
            deepest_parent = deepest_parent.max(entry.lineage_depth);
        }
        if shortest_expiry.is_some_and(|expiry| self.expires_at.as_str() > expiry)
            || self.lineage_depth != deepest_parent.saturating_add(1)
        {
            return Err(MemoryError::InvalidProposal);
        }
        for claim in &self.claims {
            if !valid_id(&claim.claim_id)
                || claim.text.is_empty()
                || claim.text.len() > 1024
                || claim.citations.is_empty()
                || claim.citations.len() > 16
                || claim.uncertainty.len() > 512
            {

                return Err(MemoryError::InvalidProposal);
            }
            for citation in &claim.citations {
                if !source_set.contains(&citation.source) {
                    return Err(MemoryError::InvalidProposal);
                }
                let entry = corpus
                    .entry(&citation.source)
                    .ok_or(MemoryError::MissingParent)?;
                if citation.start_byte >= citation.end_byte
                    || citation.end_byte > entry.content.len()
                    || std::str::from_utf8(&entry.content[..citation.start_byte]).is_err()
                    || std::str::from_utf8(&entry.content[..citation.end_byte]).is_err()
                    || sha256_hex(&entry.content[citation.start_byte..citation.end_byte])
                        != citation.quote_sha256
                {
                    return Err(MemoryError::InvalidProposal);
                }
                if self.kind == ProposalKind::Extractive
                    && claim.support == ClaimSupport::Extractive
                    && !claim.citations.iter().any(|other| {
                        corpus.entry(&other.source).is_some_and(|source| {
                            std::str::from_utf8(&source.content[other.start_byte..other.end_byte])
                                .is_ok_and(|quote| quote == claim.text)
                        })
                    })
                {
                    return Err(MemoryError::InvalidProposal);
                }
            }
        }
        Ok(())
    }
}
