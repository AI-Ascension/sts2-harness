// SPDX-License-Identifier: MIT

pub const MEMORY_REVIEW_LEDGER_SCHEMA: &str = "ascension.context-memory.review-ledger.v1";
pub const MAX_REVIEW_RECORDS: usize = 512;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImmutableReviewLedger {
    reviews: BTreeMap<(String, u64), MemoryReview>,
    latest_versions: BTreeMap<String, u64>,
}

impl ImmutableReviewLedger {
    pub fn new() -> Self {
        Self {
            reviews: BTreeMap::new(),
            latest_versions: BTreeMap::new(),
        }
    }

    pub fn record(
        &mut self,
        proposal: &MemoryProposal,
        review: MemoryReview,
        corpus: &MemoryCorpus,
    ) -> Result<(), MemoryError> {
        review.validate(proposal, corpus)?;
        let key = (proposal.proposal_id.clone(), proposal.version);
        if self.reviews.contains_key(&key) {
            return Err(MemoryError::Conflict);
        }
        if self.reviews.len() >= MAX_REVIEW_RECORDS {
            return Err(MemoryError::Capacity);
        }
        self.reviews.insert(key, review);
        self.latest_versions
            .entry(proposal.proposal_id.clone())
            .and_modify(|version| *version = (*version).max(proposal.version))
            .or_insert(proposal.version);
        Ok(())
    }

    pub fn revise(
        &mut self,
        proposal: &MemoryProposal,
        content: Vec<u8>,
        corpus: &MemoryCorpus,
        now: &str,
    ) -> Result<MemoryProposal, MemoryError> {
        proposal.validate_against(corpus, now)?;
        if content.is_empty() || content.len() > MAX_SUMMARY_OUTPUT_BYTES {
            return Err(MemoryError::InvalidProposal);
        }
        let version = self
            .latest_versions
            .get(&proposal.proposal_id)
            .copied()
            .unwrap_or(proposal.version)
            .max(proposal.version)
            .saturating_add(1);
        let mut revised = proposal.clone();
        revised.version = version;
        revised.content = content;
        revised.byte_length = revised.content.len();
        revised.sha256 = sha256_hex(&revised.content);
        revised.content_ref = format!("{}-v{version}", proposal.content_ref);
        revised.status = ProposalStatus::Generated;
        revised.applied = false;
        revised.validate_against(corpus, now)?;
        self.latest_versions
            .insert(proposal.proposal_id.clone(), version);
        Ok(revised)
    }

    pub fn review(&self, proposal_id: &str, version: u64) -> Option<&MemoryReview> {
        self.reviews.get(&(proposal_id.to_owned(), version))
    }
}

impl Default for ImmutableReviewLedger {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CriticalFact {
    pub source: MemoryRef,
    pub literal: String,
}

pub fn verify_critical_facts(
    proposal: &MemoryProposal,
    corpus: &MemoryCorpus,
    facts: &[CriticalFact],
) -> Result<(), MemoryError> {
    let output = std::str::from_utf8(&proposal.content).map_err(|_| MemoryError::InvalidProposal)?;
    for fact in facts {
        if fact.literal.is_empty() || fact.literal.len() > 256 {
            return Err(MemoryError::InvalidProposal);
        }
        let source = corpus.entry(&fact.source).ok_or(MemoryError::MissingParent)?;
        let source_text = std::str::from_utf8(&source.content)
            .map_err(|_| MemoryError::InvalidProposal)?;
        if !contains_literal(source_text, &fact.literal)
            || !contains_literal(output, &fact.literal)
            || !proposal.sources.contains(&fact.source)
        {
            return Err(MemoryError::InvalidProposal);
        }
    }
    Ok(())
}

fn contains_literal(text: &str, literal: &str) -> bool {
    let mut offset = 0;
    while let Some(relative) = text[offset..].find(literal) {
        let start = offset + relative;
        let end = start + literal.len();
        let before = text[..start].chars().next_back();
        let after = text[end..].chars().next();
        if !before.is_some_and(char::is_alphanumeric)
            && !after.is_some_and(char::is_alphanumeric)
        {
            return true;
        }
        offset = end;
        if offset >= text.len() {
            break;
        }
    }
    false
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RealizedSelection {
    pub retrieval: RetrievalResponse,
    pub selection: SelectionManifest,
}

/// Realize an approved per-decision policy once.  The resulting manifest is immutable input for
/// the caller; this helper never generates a summary or mutates the corpus.
#[allow(clippy::too_many_arguments)]
pub fn realize_policy(
    corpus: &MemoryCorpus,
    policy: &MemoryPolicy,
    selection_id: impl Into<String>,
    query: &MemoryQuery,
    mandatory_bytes: Vec<u8>,
    phase2_prepared_manifest_sha256: impl Into<String>,
    prepared_content_ref: impl Into<String>,
    expires_at: impl Into<String>,
    now: &str,
) -> Result<RealizedSelection, MemoryError> {
    policy.validate(corpus)?;
    query.validate()?;
    if query.scope != policy.scope
        || query.branch_id.is_empty()
        || query.corpus_generation != policy.corpus_generation
        || query.ranker_version != policy.ranker_version
    {
        return Err(MemoryError::InvalidQuery);
    }
    let retrieval = corpus.retrieve(query, now)?;
    let optional_sources = retrieval
        .results
        .iter()
        .map(|result| result.source.clone())
        .collect::<Vec<_>>();
    let request = SelectionRequest {
        selection_id: selection_id.into(),
        policy: policy.clone(),
        branch_id: query.branch_id.clone(),
        cutoff: query.cutoff,
        corpus_generation: query.corpus_generation,
        mandatory_manifest_sha256: sha256_hex(&mandatory_bytes),
        mandatory_bytes,
        optional_sources,
        pinned_entry_ids: Vec::new(),
        prepared_content_ref: prepared_content_ref.into(),
        expires_at: expires_at.into(),
        phase2_prepared_manifest_sha256: phase2_prepared_manifest_sha256.into(),
    };
    let selection = corpus.select(&request, now)?;
    Ok(RealizedSelection { retrieval, selection })
}
