// SPDX-License-Identifier: MIT

//! Port and bounded metadata support for the context owner contract.

use super::*;
use crate::context_control::ContextSourceDocument;

include!("context_owner_support_metadata.rs");

pub trait ContextOwnerPort: Send + Sync {
    fn catalog(&self, actor: &AuthContext) -> Result<ContextBindingCatalog, ManagementError>;

    fn bind(
        &self,
        actor: &AuthContext,
        request: &ContextBindingRequest,
    ) -> Result<ContextOwnerBinding, ManagementError>;

    fn association(
        &self,
        _actor: &AuthContext,
        _snapshot: &RunSnapshot,
    ) -> Result<ContextOwnerBinding, ManagementError> {
        Err(ManagementError::unavailable(
            "context_owner_association_unavailable",
            "context owner association is not attached to this workflow owner",
        ))
    }

    fn source_status(
        &self,
        _actor: &AuthContext,
        _snapshot: &RunSnapshot,
    ) -> Result<ContextOwnerSourceStatus, ManagementError> {
        Err(ManagementError::unavailable(
            "context_owner_source_status_unavailable",
            "current context source status is not attached to this owner",
        ))
    }

    fn control(
        &self,
        _actor: &AuthContext,
        _binding: &ContextOwnerBinding,
        _command: &ContextControlCommand,
    ) -> Result<ContextControlReceipt, ManagementError> {
        Err(ManagementError::unavailable(
            "context_owner_control_unavailable",
            "context control receipt delegation is not attached to this owner",
        ))
    }

    /// Looks up the receipt the owner already issued for `command`, for callers
    /// recovering from a lost or ambiguous reply. Implementations must not
    /// re-issue, re-apply or infer an effect: `Ok(None)` means the owner has no
    /// recorded receipt for this exact command. Owners that do not advertise
    /// `receipt_recovery` in their binding continuity must not be called.
    fn control_receipt(
        &self,
        _actor: &AuthContext,
        _binding: &ContextOwnerBinding,
        _command: &ContextControlCommand,
    ) -> Result<Option<ContextControlReceipt>, ManagementError> {
        Err(ManagementError::unavailable(
            "context_owner_receipt_recovery_unavailable",
            "context control receipt recovery is not attached to this owner",
        ))
    }

    /// Persists an explicitly published immutable source for a run. Source content is encrypted
    /// by the owner and returned only to the owner-local render port.
    fn publish_source(
        &self,
        _actor: &AuthContext,
        _snapshot: &RunSnapshot,
        _source_id: &str,
        _document: &ContextSourceDocument,
    ) -> Result<ContextBindingSource, ManagementError> {
        Err(ManagementError::unavailable(
            "context_source_publication_unavailable",
            "owner context source publication is not attached",
        ))
    }

    /// Explicitly adopts a previously published source as the next durable authority revision.
    fn adopt_source(
        &self,
        _actor: &AuthContext,
        _snapshot: &RunSnapshot,
        _source_id: &str,
        _request: &ContextSourceAdoptionRequest,
    ) -> Result<ContextControlReceipt, ManagementError> {
        Err(ManagementError::unavailable(
            "context_source_adoption_unavailable",
            "owner context source adoption is not attached",
        ))
    }

    fn eligible_items(
        &self,
        _actor: &AuthContext,
        _snapshot: &RunSnapshot,
        _draft_id: Option<&str>,
        _include_content: bool,
    ) -> Result<ContextOwnerItemsView, ManagementError> {
        Err(ManagementError::unavailable(
            "context_owner_items_unavailable",
            "durable owner item lookup is not attached",
        ))
    }

    fn list_drafts(
        &self,
        _actor: &AuthContext,
        _snapshot: &RunSnapshot,
    ) -> Result<ContextOwnerDraftListView, ManagementError> {
        Err(ManagementError::unavailable(
            "context_owner_drafts_unavailable",
            "durable owner drafts are not attached",
        ))
    }

    fn get_draft(
        &self,
        _actor: &AuthContext,
        _snapshot: &RunSnapshot,
        _draft_id: &str,
    ) -> Result<ContextOwnerDraftEnvelope, ManagementError> {
        Err(ManagementError::unavailable(
            "context_owner_drafts_unavailable",
            "durable owner drafts are not attached",
        ))
    }

    fn create_draft(
        &self,
        _actor: &AuthContext,
        _snapshot: &RunSnapshot,
        _request: &ContextOwnerDraftCreateRequest,
    ) -> Result<ContextOwnerMutationReceipt, ManagementError> {
        Err(ManagementError::unavailable(
            "context_owner_drafts_unavailable",
            "durable owner draft mutations are not attached",
        ))
    }

    fn patch_draft(
        &self,
        _actor: &AuthContext,
        _snapshot: &RunSnapshot,
        _request: &ContextOwnerDraftPatchRequest,
    ) -> Result<ContextOwnerMutationReceipt, ManagementError> {
        Err(ManagementError::unavailable(
            "context_owner_drafts_unavailable",
            "durable owner draft mutations are not attached",
        ))
    }

    fn list_revisions(
        &self,
        _actor: &AuthContext,
        _snapshot: &RunSnapshot,
        _after_revision_id: Option<&str>,
        _limit: u64,
    ) -> Result<ContextOwnerRevisionPage, ManagementError> {
        Err(ManagementError::unavailable(
            "context_owner_revisions_unavailable",
            "durable owner revisions are not attached",
        ))
    }

    fn get_revision(
        &self,
        _actor: &AuthContext,
        _snapshot: &RunSnapshot,
        _revision_id: &str,
    ) -> Result<ContextOwnerRevisionEnvelope, ManagementError> {
        Err(ManagementError::unavailable(
            "context_owner_revisions_unavailable",
            "durable owner revisions are not attached",
        ))
    }

    fn create_preview(
        &self,
        _actor: &AuthContext,
        _snapshot: &RunSnapshot,
        _request: &ContextOwnerPreviewRequest,
    ) -> Result<ContextOwnerMutationReceipt, ManagementError> {
        Err(ManagementError::unavailable(
            "context_owner_preview_unavailable",
            "trusted owner render input is unavailable",
        ))
    }

    fn get_preview(
        &self,
        _actor: &AuthContext,
        _snapshot: &RunSnapshot,
        _preview_id: &str,
    ) -> Result<ContextOwnerPreviewEnvelope, ManagementError> {
        Err(ManagementError::unavailable(
            "context_owner_preview_unavailable",
            "durable owner preview is not attached",
        ))
    }

    fn publish_draft(
        &self,
        _actor: &AuthContext,
        _snapshot: &RunSnapshot,
        _request: &ContextOwnerDraftPublicationRequest,
    ) -> Result<ContextOwnerDraftPublicationReceipt, ManagementError> {
        Err(ManagementError::unavailable(
            "context_owner_publication_unavailable",
            "immutable draft publication is not attached to this owner",
        ))
    }

    fn published_sources(
        &self,
        _actor: &AuthContext,
        _snapshot: &RunSnapshot,
    ) -> Result<ContextOwnerPublishedSourcesView, ManagementError> {
        Err(ManagementError::unavailable(
            "context_owner_publication_unavailable",
            "published context sources are not attached to this owner",
        ))
    }

    fn recover_publication_receipt(
        &self,
        _actor: &AuthContext,
        _snapshot: &RunSnapshot,
        _request: &ContextOwnerDraftPublicationLookupRequest,
    ) -> Result<Option<ContextOwnerDraftPublicationReceipt>, ManagementError> {
        Err(ManagementError::unavailable(
            "context_owner_publication_recovery_unavailable",
            "exact publication receipt recovery is not attached to this owner",
        ))
    }

    fn recover_mutation_receipt(
        &self,
        _actor: &AuthContext,
        _snapshot: &RunSnapshot,
        _request: &ContextOwnerMutationRequest,
    ) -> Result<Option<ContextOwnerMutationReceipt>, ManagementError> {
        Err(ManagementError::unavailable(
            "context_owner_mutation_recovery_unavailable",
            "exact owner mutation receipt recovery is not attached",
        ))
    }

    fn render_required(&self) -> bool {
        false
    }

    /// Recovers historical receipt evidence without asserting a current association.
    ///
    /// The default adapter preserves existing owners by asking for a current
    /// association first. Owners with durable receipt history can override this
    /// method to return the original binding after restart; callers must still
    /// validate it against the admitted run and exact command.
    fn recover_control_receipt(
        &self,
        actor: &AuthContext,
        snapshot: &RunSnapshot,
        command: &ContextControlCommand,
    ) -> Result<Option<ContextControlReceiptRecovery>, ManagementError> {
        let binding = self.association(actor, snapshot)?;
        if !binding.continuity.receipt_recovery {
            return Err(ManagementError::unavailable(
                "context_control_receipt_recovery_unsupported",
                "the authoritative context owner does not advertise control receipt recovery",
            ));
        }
        let receipt = self.control_receipt(actor, &binding, command)?;
        Ok(receipt.map(|receipt| ContextControlReceiptRecovery { binding, receipt }))
    }

    fn is_available(&self) -> bool {
        true
    }
}

pub struct UnavailableContextOwnerPort;

impl ContextOwnerPort for UnavailableContextOwnerPort {
    fn catalog(&self, _actor: &AuthContext) -> Result<ContextBindingCatalog, ManagementError> {
        Err(ManagementError::unavailable(
            "context_owner_catalog_unavailable",
            "authoritative context owner catalog is not attached",
        ))
    }

    fn bind(
        &self,
        _actor: &AuthContext,
        _request: &ContextBindingRequest,
    ) -> Result<ContextOwnerBinding, ManagementError> {
        Err(ManagementError::unavailable(
            "context_owner_binding_unavailable",
            "authoritative context owner is not attached",
        ))
    }

    fn control_receipt(
        &self,
        _actor: &AuthContext,
        _binding: &ContextOwnerBinding,
        _command: &ContextControlCommand,
    ) -> Result<Option<ContextControlReceipt>, ManagementError> {
        Err(ManagementError::unavailable(
            "context_owner_receipt_recovery_unavailable",
            "authoritative context owner is not attached",
        ))
    }

    fn is_available(&self) -> bool {
        false
    }
}
