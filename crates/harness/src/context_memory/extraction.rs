// SPDX-License-Identifier: MIT

impl MemoryCorpus {
    /// Produce an explicitly lossy extract when a caller supplies a smaller byte budget.  The
    /// omission is retained in the proposal so this path can never be mistaken for a lossless
    /// summary.
    #[allow(clippy::too_many_arguments)]
    pub fn bounded_extract(
        &self,
        proposal_id: impl Into<String>,
        source_ref: &MemoryRef,
        branch_id: &str,
        cutoff: u64,
        corpus_generation: u64,
        max_bytes: usize,
        now: &str,
        created_at: impl Into<String>,
        expires_at: impl Into<String>,
    ) -> Result<MemoryProposal, MemoryError> {
        if max_bytes == 0 || max_bytes > MAX_SUMMARY_OUTPUT_BYTES {
            return Err(MemoryError::BudgetExceeded);
        }
        let entry = self.entries.get(source_ref).ok_or(MemoryError::MissingParent)?;
        self.eligible_entry(entry, branch_id, cutoff, corpus_generation, now, false)?;
        let mut end = entry.content.len().min(max_bytes);
        while end > 0 && std::str::from_utf8(&entry.content[..end]).is_err() {
            end = end.saturating_sub(1);
        }
        if end == 0 {
            return Err(MemoryError::BudgetExceeded);
        }
        let text = std::str::from_utf8(&entry.content[..end])
            .map_err(|_| MemoryError::InvalidProposal)?
            .to_owned();
        let proposal_id = proposal_id.into();
        let proposal = MemoryProposal {
            schema: MEMORY_PROPOSAL_SCHEMA.to_owned(),
            proposal_id: proposal_id.clone(),
            version: 1,
            scope: self.scope.clone(),
            branch_id: branch_id.to_owned(),
            kind: ProposalKind::Extractive,
            sources: vec![source_ref.clone()],
            cutoff,
            corpus_generation,
            claims: vec![MemoryClaim {
                claim_id: format!("{proposal_id}-claim-0"),
                text,
                support: ClaimSupport::Extractive,
                citations: vec![Citation {
                    source: source_ref.clone(),
                    start_byte: 0,
                    end_byte: end,
                    quote_sha256: sha256_hex(&entry.content[..end]),
                }],
                uncertainty: "bounded extract omits source bytes".to_owned(),
                applicability: Applicability::Historical,
            }],
            omissions: (end < entry.content.len())
                .then(|| "source bytes after the bounded extract".to_owned())
                .into_iter()
                .collect(),
            contradictions: Vec::new(),
            lineage_depth: 1,
            status: ProposalStatus::MachineChecked,
            content_ref: format!("{proposal_id}-content"),
            sha256: sha256_hex(&entry.content[..end]),
            byte_length: end,
            source_reconstruction: if end < entry.content.len() {
                SourceReconstruction::Partial
            } else {
                SourceReconstruction::Available
            },
            created_at: created_at.into(),
            expires_at: expires_at.into(),
            applied: false,
            content: entry.content[..end].to_vec(),
        };
        proposal.validate_against(self, now)?;
        Ok(proposal)
    }
}
