// SPDX-License-Identifier: MIT

//! The bounded, proposal-only authoring-inference endpoint (`sts2-harness#105`).
//!
//! The endpoint contacts one explicitly attached provider port, admits the
//! result through the owner compiler and the served catalogs, and returns a
//! *proposal*. It never writes, publishes or runs a definition and never
//! reaches a game instance; a proposal is data a consumer may later choose to
//! apply through the separate Studio draft authority.
//!
//! The order is admission-before-reservation: the base revision, the served
//! catalogs and the pinned profile are admitted first, then one operation is
//! reserved in the journal, and only then is the provider contacted. The base
//! is re-admitted after the provider call, so a concurrent draft change loses
//! the proposal to an actionable conflict instead of overwriting the winner.

#[path = "service_authoring_inference_admission.rs"]
mod admission;

use super::super::authoring_inference::{
    AuthoringInferenceBegin, AuthoringInferenceProviderRequest, admitted_capabilities,
    authoring_inference_operation_id,
};
use super::super::contract::digest_value;
use super::super::contract_authoring::StudioDraftRecord;
use super::super::contract_authoring_inference::{
    AuthoringInferenceCost, AuthoringInferenceOperationRecord, AuthoringInferenceOperationState,
    AuthoringInferenceProposal, AuthoringInferenceRequest,
};
use super::support::authorize;
use super::{AuthContext, ManagementError, ManagementService};

use admission::{
    ProposalContext, admit_base, admit_candidate, admit_manifest, admit_pinned_profile,
    draft_digest, encode_error, replay, seal_proposal,
};

impl ManagementService {
    /// Returns one proposal for `POST /v1/studio/authoring-inference`.
    pub fn authoring_inference_proposal(
        &self,
        actor: &AuthContext,
        request: AuthoringInferenceRequest,
    ) -> Result<AuthoringInferenceProposal, ManagementError> {
        authorize(actor, "workflow:author", None)?;
        request.validate()?;
        let capabilities = self.capabilities.capabilities()?;
        let manifest_digest = digest_value(&capabilities)?;
        admit_manifest(&manifest_digest, &request)?;
        let catalog = self
            .capabilities
            .inference_profile_catalog(actor)?
            .ok_or_else(|| {
                ManagementError::unavailable(
                    "authoring_inference_catalog_unavailable",
                    "authoring inference requires an owner-served inference-profile catalog",
                )
            })?;
        catalog.validate()?;
        if catalog.catalog_digest != request.catalogs.inference_catalog_digest {
            return Err(ManagementError::conflict(
                "authoring_inference_catalog_stale",
                "the authored inference catalog digest is not the served revision",
            ));
        }
        admit_pinned_profile(&catalog, &request.inference_profile_ref)?;
        let base = admit_base(&self.authoring_stored_draft(&request)?, &request)?;

        let request_digest = digest_value(&serde_json::to_value(&request).map_err(encode_error)?)?;
        let operation_id =
            authoring_inference_operation_id(&request.base.draft_id, &request.client_mutation_id);
        let reserved = AuthoringInferenceCost {
            provider_calls: 1,
            output_tokens: 0,
        };
        match self.authoring_inference_journal.begin(
            &operation_id,
            &request.base.draft_id,
            &request.client_mutation_id,
            &request_digest,
            reserved,
        )? {
            AuthoringInferenceBegin::Replayed(record) => return replay(&record),
            AuthoringInferenceBegin::InProgress(record) => {
                return Err(ManagementError::conflict(
                    "authoring_inference_operation_in_progress",
                    format!(
                        "operation {} is already reserved and unresolved",
                        record.operation_id
                    ),
                ));
            }
            AuthoringInferenceBegin::Conflict(record) => {
                return Err(ManagementError::conflict(
                    "authoring_inference_operation_conflict",
                    format!(
                        "operation {} is held by a different request",
                        record.operation_id
                    ),
                ));
            }
            AuthoringInferenceBegin::Started(_) => {}
        }

        let provider_request = AuthoringInferenceProviderRequest {
            requirement_summary: request.requirement.summary.clone(),
            max_stages: request.requirement.max_stages,
            max_candidate_bytes: request.budget.max_candidate_bytes,
            admitted_capabilities: admitted_capabilities(&capabilities),
        };
        let candidate = match self.authoring_inference_provider.propose(&provider_request) {
            Ok(candidate) => candidate,
            Err(error) if error.code == "authoring_inference_cancelled" => {
                self.complete(
                    &operation_id,
                    AuthoringInferenceOperationState::Cancelled,
                    reserved,
                    None,
                    &error.message,
                )?;
                return Err(error);
            }
            Err(error) => {
                self.complete(
                    &operation_id,
                    AuthoringInferenceOperationState::Unknown,
                    reserved,
                    None,
                    &error.code,
                )?;
                return Err(error);
            }
        };

        let cost = AuthoringInferenceCost {
            provider_calls: candidate.cost.provider_calls,
            output_tokens: candidate.cost.output_tokens,
        };
        admit_candidate(
            self.definitions.as_ref(),
            &request,
            &capabilities,
            &catalog,
            &candidate.definition,
        )
        .inspect_err(|error| {
            let _ = self.complete(
                &operation_id,
                AuthoringInferenceOperationState::Refused,
                cost,
                None,
                &error.code,
            );
        })?;
        if cost.provider_calls > reserved.provider_calls
            || cost.output_tokens > request.budget.max_output_tokens
        {
            let error = ManagementError::budget(
                "authoring_inference_budget_exhausted",
                "the candidate exceeded its declared generation budget",
            );
            self.complete(
                &operation_id,
                AuthoringInferenceOperationState::BudgetExhausted,
                cost,
                None,
                &error.message,
            )?;
            return Err(error);
        }
        // Re-admit the base after the provider call: a concurrent draft edit
        // must win, and this operation must never bind a proposal to a base the
        // owner no longer serves.
        let current = self.authoring_stored_draft(&request)?;
        if current.revision != base.revision
            || current.etag != base.etag
            || draft_digest(&current.document)? != base.definition_digest
        {
            let error = ManagementError::conflict(
                "authoring_inference_base_conflict",
                "the draft changed while the proposal was generated; re-author against the served revision",
            );
            self.complete(
                &operation_id,
                AuthoringInferenceOperationState::Refused,
                cost,
                None,
                &error.message,
            )?;
            return Err(error);
        }
        let context = ProposalContext {
            request: &request,
            base: &base,
            request_digest: &request_digest,
            operation_id: &operation_id,
        };
        let proposal = seal_proposal(
            &context,
            candidate.definition,
            candidate.unsatisfied,
            candidate.diagnostics,
            cost,
        )?;
        self.complete(
            &operation_id,
            AuthoringInferenceOperationState::Proposed,
            cost,
            Some(Box::new(proposal.clone())),
            "proposal returned; no draft, publish or run effect",
        )?;
        Ok(proposal)
    }

    /// Reads one recorded authoring operation for
    /// `GET /v1/studio/authoring-inference/operations/{draft_id}/{client_mutation_id}`.
    pub fn authoring_inference_operation(
        &self,
        actor: &AuthContext,
        draft_id: &str,
        client_mutation_id: &str,
    ) -> Result<AuthoringInferenceOperationRecord, ManagementError> {
        authorize(actor, "workflow:read", None)?;
        super::super::contract::validate_identifier("draft_id", draft_id)?;
        super::super::contract::validate_identifier("client_mutation_id", client_mutation_id)?;
        let operation_id = authoring_inference_operation_id(draft_id, client_mutation_id);
        self.authoring_inference_journal
            .get(&operation_id)?
            .ok_or_else(|| {
                ManagementError::invalid(
                    "authoring_inference_operation_not_found",
                    "no authoring-inference operation was recorded for that identity",
                )
            })
    }

    fn authoring_stored_draft(
        &self,
        request: &AuthoringInferenceRequest,
    ) -> Result<StudioDraftRecord, ManagementError> {
        self.authoring
            .get_draft(&request.base.draft_id)?
            .ok_or_else(|| {
                ManagementError::invalid(
                    "authoring_inference_draft_not_found",
                    "the base draft was not found",
                )
            })
    }

    fn complete(
        &self,
        operation_id: &str,
        state: AuthoringInferenceOperationState,
        cost: AuthoringInferenceCost,
        proposal: Option<Box<AuthoringInferenceProposal>>,
        detail: &str,
    ) -> Result<(), ManagementError> {
        self.authoring_inference_journal
            .complete(operation_id, state, cost, proposal, detail)?;
        Ok(())
    }
}
