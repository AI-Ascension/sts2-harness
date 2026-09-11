// SPDX-License-Identifier: MIT

impl MemoryCorpus {
pub fn retrieve(
        &self,
        query: &MemoryQuery,
        now: &str,
    ) -> Result<RetrievalResponse, MemoryError> {
        query.validate()?;
        if !self.enabled {
            return Err(MemoryError::Disabled);
        }
        if query.scope != self.scope {
            return Err(MemoryError::PermissionDenied);
        }
        if query.corpus_generation > self.generation
            || !self.projection_healthy
            || self.projection_generation < query.corpus_generation
        {
            return Err(MemoryError::ProjectionUnavailable);
        }
        let terms = normalize_terms(&query.query)?;
        let mut ranked = Vec::new();
        let mut excluded = Vec::new();
        for entry in self.entries.values() {
            if self
                .eligible_entry(
                    entry,
                    &query.branch_id,
                    query.cutoff,
                    query.corpus_generation,
                    now,
                    false,
                )
                .is_err()
            {
                continue;
            }
            let text =
                std::str::from_utf8(&entry.content).map_err(|_| MemoryError::InvalidEntry)?;
            let normalized = normalize_document_terms(text);
            let score = terms
                .iter()
                .map(|term| {
                    normalized
                        .iter()
                        .filter(|candidate| *candidate == term)
                        .count() as i64
                })
                .sum::<i64>();
            if score > 0 {
                ranked.push((score, entry.observed_seq, entry.entry_id.clone(), entry));
            } else {
                excluded.push(ExclusionReason {
                    entry_id: entry.entry_id.clone(),
                    reason: "no_lexical_match".to_owned(),
                });
            }
        }
        ranked.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| left.1.cmp(&right.1))
                .then_with(|| left.2.cmp(&right.2))
        });
        let truncated = ranked.len() > query.max_candidates || ranked.len() > query.limit;
        ranked.truncate(query.max_candidates.min(query.limit));
        let results = ranked
            .into_iter()
            .map(|(score, _, _, entry)| RetrievalResult {
                source: entry.reference(),
                score,
                reasons: vec!["lexical_match".to_owned(), "historical_source".to_owned()],
                snippet: bounded_snippet(&entry.content),
            })
            .collect();
        let query_id = if valid_id(&query.query_id) {
            query.query_id.clone()
        } else {
            format!("query-{}", &sha256_hex(query.query.as_bytes())[..16])
        };
        Ok(RetrievalResponse {
            schema: MEMORY_RETRIEVAL_SCHEMA.to_owned(),
            query_id,
            scope: query.scope.clone(),
            branch_id: query.branch_id.clone(),
            query_sha256: sha256_hex(query.query.as_bytes()),
            cutoff: query.cutoff,
            corpus_generation: query.corpus_generation,
            projection_generation: self.projection_generation,
            revocation_epoch: self.revocation_epoch,
            ranker_version: query.ranker_version.clone(),
            results,
            coverage: if truncated {
                RetrievalCoverage::CandidateLimited
            } else {
                RetrievalCoverage::CompleteWithinScope
            },
            inference_calls: 0,
            excluded,
        })
    }

    /// Build a deterministic extract from immutable source bytes.  Every claim cites the exact
    /// full source span; callers may later add explicitly labelled omissions without changing the
    /// source identities.
    #[allow(clippy::too_many_arguments)]
    pub fn exact_extract(
        &self,
        proposal_id: impl Into<String>,
        source_refs: &[MemoryRef],
        branch_id: &str,
        cutoff: u64,
        corpus_generation: u64,
        now: &str,
        created_at: impl Into<String>,
        expires_at: impl Into<String>,
    ) -> Result<MemoryProposal, MemoryError> {
        if source_refs.is_empty() || source_refs.len() > MAX_SOURCES_PER_JOB {
            return Err(MemoryError::InvalidProposal);
        }
        let proposal_id = proposal_id.into();
        let mut content = Vec::new();
        let mut claims = Vec::new();
        for (index, reference) in source_refs.iter().enumerate() {
            let entry = self
                .entries
                .get(reference)
                .ok_or(MemoryError::MissingParent)?;
            self.eligible_entry(entry, branch_id, cutoff, corpus_generation, now, false)?;
            if index > 0 {
                content.extend_from_slice(b"\n");
            }
            content.extend_from_slice(&entry.content);
            let text = std::str::from_utf8(&entry.content)
                .map(str::to_owned)
                .map_err(|_| MemoryError::InvalidProposal)?;
            claims.push(MemoryClaim {
                claim_id: format!("{proposal_id}-claim-{index}"),
                text,
                support: ClaimSupport::Extractive,
                citations: vec![Citation {
                    source: reference.clone(),
                    start_byte: 0,
                    end_byte: entry.content.len(),
                    quote_sha256: sha256_hex(&entry.content),
                }],
                uncertainty: "historical applicability; current state is separate".to_owned(),
                applicability: Applicability::Historical,
            });
        }
        let proposal = MemoryProposal {
            schema: MEMORY_PROPOSAL_SCHEMA.to_owned(),
            proposal_id,
            version: 1,
            scope: self.scope.clone(),
            branch_id: branch_id.to_owned(),
            kind: ProposalKind::Extractive,
            sources: source_refs.to_vec(),
            cutoff,
            corpus_generation,
            claims,
            omissions: Vec::new(),
            contradictions: Vec::new(),
            lineage_depth: 1,
            status: ProposalStatus::MachineChecked,
            content_ref: "extract-content".to_owned(),
            sha256: sha256_hex(&content),
            byte_length: content.len(),
            source_reconstruction: SourceReconstruction::Available,
            created_at: created_at.into(),
            expires_at: expires_at.into(),
            applied: false,
            content,
        };
        proposal.validate_against(self, now)?;
        Ok(proposal)
    }

    /// Admit a reviewed proposal as a derived entry.  Review and source identities are checked
    /// before publication; this function never changes an active Phase2 revision or resumes a
    /// run.
    pub fn admit_reviewed_proposal(
        &mut self,
        proposal: &MemoryProposal,
        review: &MemoryReview,
        now: &str,
    ) -> Result<AdmissionOutcome, MemoryError> {
        proposal.validate_against(self, now)?;
        review.validate(proposal, self)?;
        if review.decision != ReviewDecision::Admit
            || matches!(proposal.status, ProposalStatus::Rejected | ProposalStatus::Stale)
        {
            return Err(MemoryError::InvalidProposal);
        }
        let mut entry = MemoryEntry::new(
            self.scope.clone(),
            proposal.proposal_id.clone(),
            proposal.proposal_id.clone(),
            MemoryKind::Summary,
            EvidenceStatus::Derived,
            proposal.branch_id.clone(),
            proposal.content_ref.clone(),
            proposal.content.clone(),
            proposal.corpus_generation,
            proposal.corpus_generation,
            proposal.corpus_generation,
            proposal.created_at.clone(),
            proposal.expires_at.clone(),
            "synthetic-v1",
            false,
        );

        entry.parents = proposal.sources.iter().map(MemoryParent::from).collect();
        entry.lineage_depth = proposal.lineage_depth;
        self.admit(entry)
    }
}
