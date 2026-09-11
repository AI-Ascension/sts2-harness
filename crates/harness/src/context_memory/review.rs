// SPDX-License-Identifier: MIT

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecision {
    Admit,
    Reject,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupportCheck {
    ExactExtractChecked,
    IndependentReview,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryReview {
    pub schema: String,
    pub review_id: String,
    pub proposal_id: String,
    pub proposal_version: u64,
    pub proposal_sha256: String,
    pub scope: MemoryScope,
    pub source_manifest_sha256: String,
    pub revocation_epoch: u64,
    pub reviewer_ref: String,
    pub decision: ReviewDecision,
    pub support_check: SupportCheck,
    pub reason_codes: Vec<String>,
    pub created_at: String,
    pub creates_active_revision: bool,
}

impl MemoryReview {
    pub fn validate(
        &self,
        proposal: &MemoryProposal,
        corpus: &MemoryCorpus,
    ) -> Result<(), MemoryError> {
        if self.schema != MEMORY_REVIEW_SCHEMA
            || !valid_id(&self.review_id)
            || self.proposal_id != proposal.proposal_id
            || self.proposal_version != proposal.version
            || self.proposal_sha256 != proposal.sha256
            || self.scope != proposal.scope
            || !valid_digest(&self.source_manifest_sha256)
            || self.source_manifest_sha256 != source_manifest_digest(&proposal.sources)
            || self.revocation_epoch != corpus.revocation_epoch()
            || !valid_id(&self.reviewer_ref)
            || self.reviewer_ref == proposal.proposal_id
            || self.reason_codes.is_empty()
            || self.reason_codes.len() > 16
            || self.reason_codes.iter().any(|reason| !valid_id(reason))
            || self.reason_codes.iter().collect::<BTreeSet<_>>().len() != self.reason_codes.len()
            || !valid_timestamp(&self.created_at)
            || self.creates_active_revision
        {
            return Err(MemoryError::ReviewBinding);
        }
        if self.decision == ReviewDecision::Admit && self.support_check == SupportCheck::Failed {
            return Err(MemoryError::ReviewBinding);
        }
        Ok(())
    }
}
