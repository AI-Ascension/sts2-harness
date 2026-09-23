// SPDX-License-Identifier: MIT

// Workflow-scoped history selection, extraction and reviewed summary jobs (issue #113).
//
// This module binds the existing context-memory primitives to the workflow that owns them and
// records one receipt per operation. It adds no new authority: selection, extraction, retrieval,
// review, adoption, approval and resume keep their existing contracts and errors, and the game
// host remains the authority for what actually happened.
//
// ## Two properties the receipts make observable
//
// **Search and preview perform zero inference.** `WorkflowHistorySession::search` and
// `WorkflowHistorySession::preview` never reach a provider port. Their receipts record
// `provider_attempts = 0`, and the counter used by `WorkflowHistorySession::generate` is a
// wrapper around the very provider the generation path calls, so a test can assert the count is
// unchanged after a search or preview.
//
// **Generation is explicit, bounded and never retried blind.** Generation requires the caller to
// pass `allow_generation = true`; a provider failure leaves the job in `outcome_unknown`, records
// an `outcome_unknown` receipt, and a later attempt must not call the provider a second time.
//
// ## Independent receipts
//
// Review, adoption, commit-held and resume each record their own receipt and each require the
// receipt of the step that authorizes it, so a consumer can see that a resume followed a held
// commit that followed an adoption that followed an independent review.

/// Stable schema identity of a workflow-scoped history session receipt.
pub const WORKFLOW_HISTORY_RECEIPT_SCHEMA: &str =
    "ascension.context-memory.workflow-history-receipt.v1";

/// Upper bound on receipts retained by one session.
pub const MAX_WORKFLOW_HISTORY_RECEIPTS: usize = 512;

/// The workflow run that owns a history session.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowHistoryOwner {
    pub workflow_id: String,
    pub scope: MemoryScope,
    pub branch_id: String,
}

impl WorkflowHistoryOwner {
    pub fn new(workflow_id: impl Into<String>, scope: MemoryScope, branch_id: impl Into<String>) -> Self {
        Self {
            workflow_id: workflow_id.into(),
            scope,
            branch_id: branch_id.into(),
        }
    }

    pub fn validate(&self) -> Result<(), MemoryError> {
        if !valid_id(&self.workflow_id) || !self.scope.valid() || !valid_id(&self.branch_id) {
            return Err(MemoryError::InvalidScope);
        }
        Ok(())
    }
}

/// The distinct operations a workflow history session records.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowHistoryOperation {
    Search,
    Preview,
    Extraction,
    Generation,
    Review,
    Adoption,
    CommitHeld,
    Resume,
}

impl WorkflowHistoryOperation {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Search => "search",
            Self::Preview => "preview",
            Self::Extraction => "extraction",
            Self::Generation => "generation",
            Self::Review => "review",
            Self::Adoption => "adoption",
            Self::CommitHeld => "commit-held",
            Self::Resume => "resume",
        }
    }
}

/// How one operation ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptOutcome {
    Completed,
    ZeroInference,
    OutcomeUnknown,
    Refused,
}

/// One immutable receipt for a workflow-scoped history operation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowHistoryReceipt {
    pub schema: String,
    pub receipt_id: String,
    pub workflow_id: String,
    pub operation: WorkflowHistoryOperation,
    pub subject_id: String,
    pub outcome: ReceiptOutcome,
    pub provider_attempts: u32,
    pub inference_calls: u32,
    pub created_at: String,
}

impl WorkflowHistoryReceipt {
    pub fn validate(&self) -> Result<(), MemoryError> {
        if self.schema != WORKFLOW_HISTORY_RECEIPT_SCHEMA
            || !valid_id(&self.receipt_id)
            || !valid_id(&self.workflow_id)
            || !valid_id(&self.subject_id)
            || !valid_timestamp(&self.created_at)
            || (self.operation != WorkflowHistoryOperation::Generation
                && (self.provider_attempts != 0 || self.inference_calls != 0))
        {
            return Err(MemoryError::InvalidQuery);
        }
        Ok(())
    }
}

/// Wraps the provider port so a caller can observe how many times generation was actually
/// attempted. The wrapper forwards one call per `generate`, so its count is the real number of
/// provider attempts made through this boundary.
pub struct CountingSummaryProvider<'provider> {
    inner: &'provider mut dyn SummaryProvider,
    attempts: u32,
}

impl<'provider> CountingSummaryProvider<'provider> {
    pub fn new(inner: &'provider mut dyn SummaryProvider) -> Self {
        Self { inner, attempts: 0 }
    }

    #[must_use]
    pub fn attempts(&self) -> u32 {
        self.attempts
    }
}

impl SummaryProvider for CountingSummaryProvider<'_> {
    fn generate(
        &mut self,
        request: SummaryGenerationRequest,
    ) -> Result<SummaryGeneration, MemoryError> {
        self.attempts = self.attempts.saturating_add(1);
        self.inner.generate(request)
    }
}

/// One workflow-scoped history session over an owned corpus.
pub struct WorkflowHistorySession {
    owner: WorkflowHistoryOwner,
    corpus: MemoryCorpus,
    receipts: Vec<WorkflowHistoryReceipt>,
}

impl WorkflowHistorySession {
    /// Bind a corpus to the workflow that owns it. A corpus for another scope is refused, so
    /// foreign material cannot be addressed through a session it does not belong to.
    pub fn new(owner: WorkflowHistoryOwner, corpus: MemoryCorpus) -> Result<Self, MemoryError> {
        owner.validate()?;
        if owner.scope != *corpus.scope() {
            return Err(MemoryError::PermissionDenied);
        }
        Ok(Self {
            owner,
            corpus,
            receipts: Vec::new(),
        })
    }

    #[must_use]
    pub fn owner(&self) -> &WorkflowHistoryOwner {
        &self.owner
    }

    #[must_use]
    pub fn corpus(&self) -> &MemoryCorpus {
        &self.corpus
    }

    #[must_use]
    pub fn receipts(&self) -> &[WorkflowHistoryReceipt] {
        &self.receipts
    }

    #[must_use]
    pub fn count(&self, operation: WorkflowHistoryOperation) -> usize {
        self.receipts
            .iter()
            .filter(|receipt| receipt.operation == operation)
            .count()
    }

    fn has(&self, operation: WorkflowHistoryOperation, subject_id: &str) -> bool {
        self.receipts
            .iter()
            .any(|receipt| receipt.operation == operation && receipt.subject_id == subject_id)
    }

    fn require(
        &self,
        operation: WorkflowHistoryOperation,
        subject_id: &str,
    ) -> Result<(), MemoryError> {
        if self.has(operation, subject_id) {
            Ok(())
        } else {
            Err(MemoryError::PermissionDenied)
        }
    }

    /// Require any one of `operations` to have been receipted for `subject_id`. Review accepts
    /// either an explicit generation or a deterministic extraction of the same proposal, so the
    /// two authorizing paths stay distinct while neither may be skipped.
    fn require_any(
        &self,
        operations: &[WorkflowHistoryOperation],
        subject_id: &str,
    ) -> Result<(), MemoryError> {
        if operations
            .iter()
            .any(|operation| self.has(*operation, subject_id))
        {
            Ok(())
        } else {
            Err(MemoryError::PermissionDenied)
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn record(
        &mut self,
        operation: WorkflowHistoryOperation,
        subject_id: &str,
        outcome: ReceiptOutcome,
        provider_attempts: u32,
        inference_calls: u32,
        now: &str,
    ) -> Result<WorkflowHistoryReceipt, MemoryError> {
        if !valid_id(subject_id) || !valid_timestamp(now) {
            return Err(MemoryError::InvalidQuery);
        }
        if self.receipts.len() >= MAX_WORKFLOW_HISTORY_RECEIPTS {
            return Err(MemoryError::Capacity);
        }
        let receipt = WorkflowHistoryReceipt {
            schema: WORKFLOW_HISTORY_RECEIPT_SCHEMA.to_owned(),
            receipt_id: format!(
                "{}-{}-{}",
                self.owner.workflow_id,
                operation.as_str(),
                self.receipts.len().saturating_add(1)
            ),
            workflow_id: self.owner.workflow_id.clone(),
            operation,
            subject_id: subject_id.to_owned(),
            outcome,
            provider_attempts,
            inference_calls,
            created_at: now.to_owned(),
        };
        receipt.validate()?;
        self.receipts.push(receipt.clone());
        Ok(receipt)
    }
}
