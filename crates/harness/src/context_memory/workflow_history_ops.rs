// SPDX-License-Identifier: MIT

// Workflow-scoped history session operations (issue #113).
//
// These methods are the workflow-facing verbs of `WorkflowHistorySession`: retrieval search,
// selection preview, deterministic extraction, explicit generation, independent review and
// adoption, held commit, and first resume. Each records one `WorkflowHistoryReceipt`, and the
// steps that depend on an earlier decision require that step's receipt first. The session type
// and its receipt vocabulary are declared in `workflow_history.rs`.

impl WorkflowHistorySession {
    /// Retrieval search. It reads the local corpus only and records a zero-inference receipt.
    pub fn search(
        &mut self,
        query: &MemoryQuery,
        now: &str,
    ) -> Result<(WorkflowHistoryReceipt, RetrievalResponse), MemoryError> {
        if query.scope != self.owner.scope {
            return Err(MemoryError::PermissionDenied);
        }
        let response = self.corpus.retrieve(query, now)?;
        let receipt = self.record(
            WorkflowHistoryOperation::Search,
            &response.query_id,
            ReceiptOutcome::ZeroInference,
            0,
            response.inference_calls,
            now,
        )?;
        Ok((receipt, response))
    }

    /// Selection preview for a bound branch. It prepares bytes only and records a
    /// zero-inference receipt.
    pub fn preview(
        &mut self,
        request: &SelectionRequest,
        now: &str,
    ) -> Result<(WorkflowHistoryReceipt, SelectionManifest), MemoryError> {
        if request.policy.scope != self.owner.scope || request.branch_id != self.owner.branch_id {
            return Err(MemoryError::PermissionDenied);
        }
        let selection = self.corpus.select(request, now)?;
        let receipt = self.record(
            WorkflowHistoryOperation::Preview,
            &selection.selection_id,
            ReceiptOutcome::ZeroInference,
            0,
            0,
            now,
        )?;
        Ok((receipt, selection))
    }

    /// Deterministic extraction over immutable, eligible sources.
    #[allow(clippy::too_many_arguments)]
    pub fn extract(
        &mut self,
        proposal_id: impl Into<String>,
        source_refs: &[MemoryRef],
        cutoff: u64,
        corpus_generation: u64,
        now: &str,
        created_at: impl Into<String>,
        expires_at: impl Into<String>,
    ) -> Result<(WorkflowHistoryReceipt, MemoryProposal), MemoryError> {
        let proposal = self.corpus.exact_extract(
            proposal_id,
            source_refs,
            &self.owner.branch_id,
            cutoff,
            corpus_generation,
            now,
            created_at,
            expires_at,
        )?;
        let receipt = self.record(
            WorkflowHistoryOperation::Extraction,
            &proposal.proposal_id,
            ReceiptOutcome::ZeroInference,
            0,
            0,
            now,
        )?;
        Ok((receipt, proposal))
    }

    /// Explicit summary generation. Requires `allow_generation`; a provider failure leaves the
    /// job `outcome_unknown` and is recorded without a retry.
    pub fn generate(
        &mut self,
        jobs: &mut SummaryJobStore,
        job_id: &str,
        provider: &mut dyn SummaryProvider,
        allow_generation: bool,
        now: &str,
    ) -> Result<(WorkflowHistoryReceipt, MemoryProposal), MemoryError> {
        // A job whose reply was lost is kept as `outcome_unknown`; refusing here means a retry
        // can never call the provider a second time, and the unknown outcome is not overwritten.
        if jobs
            .job(job_id)
            .is_some_and(|job| job.state == JobState::OutcomeUnknown)
        {
            return Err(MemoryError::JobUnknown);
        }
        let mut counting = CountingSummaryProvider::new(provider);
        let result = jobs.execute_explicit(job_id, &self.corpus, &mut counting, now, allow_generation);
        let attempts = counting.attempts();
        match result {
            Ok(proposal) => {
                let receipt = self.record(
                    WorkflowHistoryOperation::Generation,
                    &proposal.proposal_id,
                    ReceiptOutcome::Completed,
                    attempts,
                    attempts,
                    now,
                )?;
                Ok((receipt, proposal))
            }
            Err(error) => {
                if jobs
                    .job(job_id)
                    .is_some_and(|job| job.state == JobState::OutcomeUnknown)
                {
                    self.record(
                        WorkflowHistoryOperation::Generation,
                        job_id,
                        ReceiptOutcome::OutcomeUnknown,
                        attempts,
                        attempts,
                        now,
                    )?;
                }
                Err(error)
            }
        }
    }

    /// Record an independent review of a generated proposal.
    pub fn review(
        &mut self,
        review: &MemoryReview,
        proposal: &MemoryProposal,
        now: &str,
    ) -> Result<WorkflowHistoryReceipt, MemoryError> {
        review.validate(proposal, &self.corpus)?;
        self.require_any(
            &[
                WorkflowHistoryOperation::Generation,
                WorkflowHistoryOperation::Extraction,
            ],
            &proposal.proposal_id,
        )?;
        let outcome = if review.decision == ReviewDecision::Admit {
            ReceiptOutcome::Completed
        } else {
            ReceiptOutcome::Refused
        };
        self.record(
            WorkflowHistoryOperation::Review,
            &review.review_id,
            outcome,
            0,
            0,
            now,
        )
    }

    /// Adopt a reviewed proposal as derived memory. Requires the independent review receipt.
    pub fn adopt(
        &mut self,
        proposal: &MemoryProposal,
        review: &MemoryReview,
        now: &str,
    ) -> Result<(WorkflowHistoryReceipt, AdmissionOutcome), MemoryError> {
        self.require(WorkflowHistoryOperation::Review, &review.review_id)?;
        let outcome = self.corpus.admit_reviewed_proposal(proposal, review, now)?;
        let receipt = self.record(
            WorkflowHistoryOperation::Adoption,
            &proposal.proposal_id,
            ReceiptOutcome::Completed,
            0,
            0,
            now,
        )?;
        Ok((receipt, outcome))
    }

    /// Commit-held approval, recorded as its own receipt.
    pub fn commit_held(
        &mut self,
        approvals: &mut ApprovalStore,
        approval_id: &str,
        current_revocation_epoch: u64,
        now: &str,
    ) -> Result<(WorkflowHistoryReceipt, MemoryApproval), MemoryError> {
        let approval = approvals.commit_held(approval_id, current_revocation_epoch, now)?;
        let receipt = self.record(
            WorkflowHistoryOperation::CommitHeld,
            approval_id,
            ReceiptOutcome::Completed,
            0,
            0,
            now,
        )?;
        Ok((receipt, approval))
    }

    /// First resume of a prepared, held approval. Requires the held-commit receipt, so a resume
    /// cannot precede the commit that authorized it.
    #[allow(clippy::too_many_arguments)]
    pub fn resume(
        &mut self,
        resumes: &mut FirstResumeLedger,
        approvals: &ApprovalStore,
        approval_id: &str,
        current_revocation_epoch: u64,
        now: &str,
        rendered_bytes: &[u8],
    ) -> Result<(WorkflowHistoryReceipt, ResumeSubmission), MemoryError> {
        self.require(WorkflowHistoryOperation::CommitHeld, approval_id)?;
        let approval = approvals
            .approval(approval_id)
            .ok_or(MemoryError::StaleApproval)?;
        let (outcome, submission) =
            resumes.submit_first(approval, current_revocation_epoch, now, rendered_bytes)?;
        let receipt_outcome = match outcome {
            ResumeOutcome::Submitted => ReceiptOutcome::Completed,
            ResumeOutcome::AlreadySubmitted => ReceiptOutcome::Refused,
        };
        let receipt = self.record(
            WorkflowHistoryOperation::Resume,
            approval_id,
            receipt_outcome,
            0,
            0,
            now,
        )?;
        Ok((receipt, submission))
    }
}
