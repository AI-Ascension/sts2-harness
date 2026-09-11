// SPDX-License-Identifier: MIT

#[derive(Clone, Debug)]
pub struct SummaryJobStore {
    jobs: BTreeMap<String, SummaryJob>,
    by_idempotency: BTreeMap<String, String>,
    max_jobs: usize,
}

impl SummaryJobStore {
    pub fn new(max_jobs: usize) -> Result<Self, MemoryError> {
        if max_jobs == 0 || max_jobs > 32 {
            return Err(MemoryError::Capacity);
        }
        Ok(Self {
            jobs: BTreeMap::new(),
            by_idempotency: BTreeMap::new(),
            max_jobs,
        })
    }

    pub fn admit(&mut self, job: SummaryJob) -> Result<bool, MemoryError> {
        job.validate()?;
        if let Some(existing_id) = self.by_idempotency.get(&job.idempotency_key) {
            let existing = self.jobs.get(existing_id).ok_or(MemoryError::JobConflict)?;
            if existing == &job {
                return Ok(false);
            }
            return Err(MemoryError::JobConflict);
        }
        if self.jobs.len() >= self.max_jobs {
            return Err(MemoryError::Capacity);
        }
        self.by_idempotency
            .insert(job.idempotency_key.clone(), job.job_id.clone());
        self.jobs.insert(job.job_id.clone(), job);
        Ok(true)
    }

    pub fn mark_unknown(
        &mut self,
        job_id: &str,
        attempt_id: impl Into<String>,
    ) -> Result<(), MemoryError> {
        let job = self.jobs.get_mut(job_id).ok_or(MemoryError::JobUnknown)?;
        job.state = JobState::OutcomeUnknown;
        job.provider_write_state = ProviderWriteState::PossiblyWritten;
        job.attempt_id = Some(attempt_id.into());
        Ok(())
    }

    pub fn retry(&self, job_id: &str) -> Result<(), MemoryError> {
        let job = self.jobs.get(job_id).ok_or(MemoryError::JobUnknown)?;
        if job.state == JobState::OutcomeUnknown {
            return Err(MemoryError::JobUnknown);
        }
        Ok(())
    }

    pub fn job(&self, job_id: &str) -> Option<&SummaryJob> {
        self.jobs.get(job_id)
    }

    /// Execute a queued job only after the caller supplies the explicit generation permission.
    /// The adapter sees bounded source bytes and no control or game callback.  Unknown outcomes
    /// are retained by `mark_unknown` and are never retried automatically.
    pub fn execute_explicit(
        &mut self,
        job_id: &str,
        corpus: &MemoryCorpus,
        provider: &mut dyn SummaryProvider,
        now: &str,
        allow_generation: bool,
    ) -> Result<MemoryProposal, MemoryError> {
        if !allow_generation {
            return Err(MemoryError::PermissionDenied);
        }
        let job = self.jobs.get_mut(job_id).ok_or(MemoryError::JobUnknown)?;
        if job.state != JobState::Queued || job.deadline_at.as_str() <= now {
            job.state = if job.deadline_at.as_str() <= now {
                JobState::Expired
            } else {
                JobState::Blocked
            };
            return Err(MemoryError::JobUnknown);
        }
        if job.scope != *corpus.scope()
            || source_manifest_digest(&job.sources) != job.source_manifest_sha256
        {
            job.state = JobState::Blocked;
            return Err(MemoryError::InvalidProposal);
        }
        let mut source_bytes = Vec::with_capacity(job.sources.len());
        let mut input_bytes = 0_usize;
        let mut shortest_expiry: Option<&str> = None;
        let mut deepest_parent = 0_u8;
        for reference in &job.sources {
            let entry = corpus.entry(reference).ok_or(MemoryError::MissingParent)?;
            corpus.eligible_entry(
                entry,
                &job.branch_id,
                job.cutoff,
                job.corpus_generation,
                now,
                false,
            )?;
            input_bytes = input_bytes.saturating_add(entry.content.len());
            shortest_expiry = Some(
                shortest_expiry
                    .map_or(entry.expires_at.as_str(), |current| {
                        current.min(entry.expires_at.as_str())
                    }),
            );
            deepest_parent = deepest_parent.max(entry.lineage_depth);
            source_bytes.push((reference.clone(), entry.content.clone()));
        }
        if input_bytes != job.input_bytes
            || shortest_expiry.is_some_and(|expiry| job.deadline_at.as_str() > expiry)
        {
            job.state = JobState::Blocked;
            return Err(MemoryError::InvalidProposal);
        }
        let first_citation = {
            let (first_source, first_bytes) = source_bytes
                .first()
                .ok_or(MemoryError::InvalidProposal)?;
            Citation {
                source: first_source.clone(),
                start_byte: 0,
                end_byte: first_bytes.len(),
                quote_sha256: sha256_hex(first_bytes),
            }
        };
        job.state = JobState::Executing;
        job.provider_write_state = ProviderWriteState::IntentPersisted;
        let request = SummaryGenerationRequest {
            job_id: job.job_id.clone(),
            generator_profile: job.generator_profile.clone(),
            prompt_sha256: job.generator_prompt_sha256.clone(),
            output_schema_sha256: job.output_schema_sha256.clone(),
            source_bytes,
            max_output_bytes: job.max_output_bytes,
        };
        let generated = match provider.generate(request) {
            Ok(generated) => generated,
            Err(error) => {
                job.state = JobState::OutcomeUnknown;
                job.provider_write_state = ProviderWriteState::PossiblyWritten;
                return Err(error);
            }
        };
        if generated.output.len() > job.max_output_bytes
            || sha256_hex(&generated.output) != generated.output_sha256
        {
            job.state = JobState::Rejected;
            return Err(MemoryError::InvalidProposal);
        }
        job.state = JobState::Succeeded;
        job.provider_write_state = ProviderWriteState::Completed;

        let proposal = MemoryProposal {
            schema: MEMORY_PROPOSAL_SCHEMA.to_owned(),
            proposal_id: format!("proposal-{}", job.job_id),
            version: 1,
            scope: job.scope.clone(),
            branch_id: job.branch_id.clone(),
            kind: ProposalKind::Abstractive,
            sources: job.sources.clone(),
            cutoff: job.cutoff,
            corpus_generation: job.corpus_generation,
            claims: vec![MemoryClaim {
                claim_id: format!("proposal-{}-claim-0", job.job_id),
                text: "Generated output is unverified and requires independent review.".to_owned(),
                support: ClaimSupport::Unknown,
                citations: vec![first_citation],
                uncertainty: "provider output is not an authority source".to_owned(),
                applicability: Applicability::Unknown,
            }],
            omissions: vec!["Abstractive output requires independent review.".to_owned()],
            contradictions: Vec::new(),
            lineage_depth: deepest_parent.saturating_add(1),
            status: ProposalStatus::ReviewRequired,
            content_ref: format!("proposal-content-{}", job.job_id),
            sha256: generated.output_sha256,
            byte_length: generated.output.len(),
            source_reconstruction: SourceReconstruction::Available,
            created_at: now.to_owned(),
            expires_at: job.deadline_at.clone(),
            applied: false,
            content: generated.output,
        };
        Ok(proposal)
    }
}
