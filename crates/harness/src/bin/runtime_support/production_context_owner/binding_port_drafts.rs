// SPDX-License-Identifier: MIT

    fn eligible_items(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        draft_id: Option<&str>,
        include_content: bool,
    ) -> Result<sts2_harness::management::ContextOwnerItemsView, ManagementError> {
        self.eligible_items_current(actor, snapshot, draft_id, include_content)
    }

    fn list_drafts(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
    ) -> Result<sts2_harness::management::ContextOwnerDraftListView, ManagementError> {
        self.list_drafts_current(actor, snapshot)
    }

    fn get_draft(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        draft_id: &str,
    ) -> Result<sts2_harness::management::ContextOwnerDraftEnvelope, ManagementError> {
        self.get_draft_current(actor, snapshot, draft_id)
    }

    fn create_draft(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        request: &sts2_harness::management::ContextOwnerDraftCreateRequest,
    ) -> Result<sts2_harness::management::ContextOwnerMutationReceipt, ManagementError> {
        self.create_draft_current(actor, snapshot, request)
    }

    fn patch_draft(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        request: &sts2_harness::management::ContextOwnerDraftPatchRequest,
    ) -> Result<sts2_harness::management::ContextOwnerMutationReceipt, ManagementError> {
        self.patch_draft_current(actor, snapshot, request)
    }

    fn list_revisions(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        after_revision_id: Option<&str>,
        limit: u64,
    ) -> Result<sts2_harness::management::ContextOwnerRevisionPage, ManagementError> {
        self.list_revisions_current(actor, snapshot, after_revision_id, limit)
    }

    fn get_revision(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        revision_id: &str,
    ) -> Result<sts2_harness::management::ContextOwnerRevisionEnvelope, ManagementError> {
        self.get_revision_current(actor, snapshot, revision_id)
    }

    fn create_preview(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        request: &sts2_harness::management::ContextOwnerPreviewRequest,
    ) -> Result<sts2_harness::management::ContextOwnerMutationReceipt, ManagementError> {
        self.create_preview_current(actor, snapshot, request)
    }

    fn get_preview(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        preview_id: &str,
    ) -> Result<sts2_harness::management::ContextOwnerPreviewEnvelope, ManagementError> {
        self.get_preview_current(actor, snapshot, preview_id)
    }

    fn recover_mutation_receipt(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        request: &sts2_harness::management::ContextOwnerMutationRequest,
    ) -> Result<Option<sts2_harness::management::ContextOwnerMutationReceipt>, ManagementError>
    {
        self.recover_mutation_receipt_current(actor, snapshot, request)
    }
