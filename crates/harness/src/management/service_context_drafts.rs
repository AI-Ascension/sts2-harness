// SPDX-License-Identifier: MIT

//! Authenticated management entry points for owner-local context drafts.

use super::super::context_owner::{
    ContextOwnerDraftCreateRequest, ContextOwnerDraftEnvelope, ContextOwnerDraftListView,
    ContextOwnerDraftOperation, ContextOwnerDraftPatchRequest, ContextOwnerItemsView,
    ContextOwnerMutationReceipt, ContextOwnerMutationRequest, ContextOwnerPreviewEnvelope,
    ContextOwnerPreviewRequest, ContextOwnerRevisionEnvelope, ContextOwnerRevisionPage,
};
use super::support::authorize;
use super::{AuthContext, ManagementError, ManagementService, RunSnapshot, validate_identifier};

impl ManagementService {
    pub fn eligible_context_owner_items(
        &self,
        actor: &AuthContext,
        run_id: &str,
        draft_id: Option<&str>,
        include_content: bool,
    ) -> Result<ContextOwnerItemsView, ManagementError> {
        let snapshot = self.context_owner_snapshot(actor, run_id, "workflow:read")?;
        if let Some(draft_id) = draft_id {
            validate_identifier("context_draft_id", draft_id)?;
        }
        if include_content {
            authorize(actor, "workflow:context:content:read", Some(run_id))?;
        }
        self.context_owner
            .eligible_items(actor, &snapshot, draft_id, include_content)
    }

    pub fn context_owner_drafts(
        &self,
        actor: &AuthContext,
        run_id: &str,
    ) -> Result<ContextOwnerDraftListView, ManagementError> {
        let snapshot = self.context_owner_snapshot(actor, run_id, "workflow:read")?;
        self.context_owner.list_drafts(actor, &snapshot)
    }

    pub fn context_owner_draft(
        &self,
        actor: &AuthContext,
        run_id: &str,
        draft_id: &str,
    ) -> Result<ContextOwnerDraftEnvelope, ManagementError> {
        let snapshot = self.context_owner_snapshot(actor, run_id, "workflow:read")?;
        validate_identifier("context_draft_id", draft_id)?;
        self.context_owner.get_draft(actor, &snapshot, draft_id)
    }

    pub fn create_context_owner_draft(
        &self,
        actor: &AuthContext,
        run_id: &str,
        request: &ContextOwnerDraftCreateRequest,
    ) -> Result<ContextOwnerMutationReceipt, ManagementError> {
        let snapshot = self.context_owner_snapshot(actor, run_id, "workflow:context:edit")?;
        self.context_owner.create_draft(actor, &snapshot, request)
    }

    pub fn patch_context_owner_draft(
        &self,
        actor: &AuthContext,
        run_id: &str,
        request: &ContextOwnerDraftPatchRequest,
    ) -> Result<ContextOwnerMutationReceipt, ManagementError> {
        validate_identifier("run_id", run_id)?;
        let changes_objective = request.operations.iter().any(|operation| {
            matches!(
                operation,
                ContextOwnerDraftOperation::SetObjective { .. }
                    | ContextOwnerDraftOperation::RemoveObjective
            )
        });
        let changes_other = request.operations.iter().any(|operation| {
            !matches!(
                operation,
                ContextOwnerDraftOperation::SetObjective { .. }
                    | ContextOwnerDraftOperation::RemoveObjective
            )
        });
        if changes_other {
            authorize(actor, "workflow:context:edit", Some(run_id))?;
        }
        if changes_objective {
            authorize(actor, "workflow:context:objective:edit", Some(run_id))?;
        }
        if !changes_other && !changes_objective {
            return Err(ManagementError::invalid(
                "context_draft_patch_empty",
                "a draft patch must contain at least one operation",
            ));
        }
        if !actor.can_run(run_id) {
            return Err(ManagementError::forbidden(
                "run_scope_forbidden",
                "actor cannot access this workflow run",
            ));
        }
        let snapshot = self.store.get_run(run_id)?.ok_or_else(|| {
            ManagementError::invalid("run_not_found", "workflow run was not found")
        })?;
        self.context_owner.patch_draft(actor, &snapshot, request)
    }

    pub fn context_owner_revisions(
        &self,
        actor: &AuthContext,
        run_id: &str,
        after_revision_id: Option<&str>,
        limit: u64,
    ) -> Result<ContextOwnerRevisionPage, ManagementError> {
        let snapshot = self.context_owner_snapshot(actor, run_id, "workflow:read")?;
        if !(1..=super::super::context_owner::MAX_CONTEXT_OWNER_PAGE_SIZE).contains(&limit) {
            return Err(ManagementError::invalid(
                "context_revision_page_limit",
                "revision page limit is outside the supported bound",
            ));
        }
        if let Some(revision_id) = after_revision_id {
            validate_identifier("context_revision_id", revision_id)?;
        }
        self.context_owner
            .list_revisions(actor, &snapshot, after_revision_id, limit)
    }

    pub fn context_owner_revision(
        &self,
        actor: &AuthContext,
        run_id: &str,
        revision_id: &str,
    ) -> Result<ContextOwnerRevisionEnvelope, ManagementError> {
        let snapshot = self.context_owner_snapshot(actor, run_id, "workflow:read")?;
        validate_identifier("context_revision_id", revision_id)?;
        self.context_owner
            .get_revision(actor, &snapshot, revision_id)
    }

    pub fn create_context_owner_preview(
        &self,
        actor: &AuthContext,
        run_id: &str,
        request: &ContextOwnerPreviewRequest,
    ) -> Result<ContextOwnerMutationReceipt, ManagementError> {
        let snapshot = self.context_owner_snapshot(actor, run_id, "workflow:context:edit")?;
        self.context_owner.create_preview(actor, &snapshot, request)
    }

    pub fn context_owner_preview(
        &self,
        actor: &AuthContext,
        run_id: &str,
        preview_id: &str,
    ) -> Result<ContextOwnerPreviewEnvelope, ManagementError> {
        let snapshot = self.context_owner_snapshot(actor, run_id, "workflow:read")?;
        validate_identifier("context_preview_id", preview_id)?;
        self.context_owner.get_preview(actor, &snapshot, preview_id)
    }

    pub fn recover_context_owner_mutation_receipt(
        &self,
        actor: &AuthContext,
        run_id: &str,
        request: &ContextOwnerMutationRequest,
    ) -> Result<Option<ContextOwnerMutationReceipt>, ManagementError> {
        let snapshot = self.context_owner_snapshot(actor, run_id, "workflow:read")?;
        self.context_owner
            .recover_mutation_receipt(actor, &snapshot, request)
    }

    fn context_owner_snapshot(
        &self,
        actor: &AuthContext,
        run_id: &str,
        scope: &str,
    ) -> Result<RunSnapshot, ManagementError> {
        validate_identifier("run_id", run_id)?;
        authorize(actor, scope, Some(run_id))?;
        self.store
            .get_run(run_id)?
            .ok_or_else(|| ManagementError::invalid("run_not_found", "workflow run was not found"))
    }
}
