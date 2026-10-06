// SPDX-License-Identifier: MIT

use super::*;
use sts2_harness::context_control::{
    DurableActiveContextSource, DurableContextOwnerPublication, DurableContextSourceSnapshot,
};

impl Owner {
    pub(super) fn active_source_metadata(
        &self,
        entry: &Current,
        actor: &AuthContext,
        binding: &ContextOwnerBinding,
    ) -> Result<Option<ContextBindingSource>, ManagementError> {
        let active_revision_id = &entry.authority.state().active_revision_id;
        let Some((active, _source)) = entry
            .store
            .active_context_source(active_revision_id)
            .map_err(publication_store_error)?
        else {
            return Ok(None);
        };
        let identity = ContextBindingSource {
            source_id: active.source_id.clone(),
            version: active.version,
            digest: active.digest.clone(),
        };
        if is_publication_source(&identity.source_id) {
            let now = unix_time()?;
            let (publication, _) =
                self.load_active_publication(entry, &actor.subject, &active, now)?;
            if !publication_binding_matches(&publication.receipt.binding, binding) {
                return Err(publication_active_conflict());
            }
        } else {
            let advertised = self.advertised_source(&identity.source_id)?;
            if advertised != identity || active.active_revision_id != *active_revision_id {
                return Err(publication_active_conflict());
            }
        }
        Ok(Some(identity))
    }

    pub(super) fn load_active_publication(
        &self,
        entry: &Current,
        actor_subject: &str,
        active: &DurableActiveContextSource,
        now: u64,
    ) -> Result<(DurableContextOwnerPublication, DurableContextSourceSnapshot), ManagementError>
    {
        if !is_publication_source(&active.source_id)
            || active.active_revision_id != entry.authority.state().active_revision_id
        {
            return Err(publication_active_conflict());
        }
        let (publication, source) = entry
            .store
            .load_publication_source(
                &self.configuration.owner_id,
                actor_subject,
                &active.source_id,
                active.version,
                &active.digest,
            )
            .map_err(publication_store_error)?
            .ok_or_else(publication_missing)?;
        let link = entry
            .store
            .load_active_publication_link(
                &self.configuration.owner_id,
                actor_subject,
                &active.source_id,
                active.version,
                &active.digest,
                &active.active_revision_id,
            )
            .map_err(publication_store_error)?
            .ok_or_else(publication_missing)?;
        let receipt = &publication.receipt;
        if receipt.workflow_run_id != entry.authority.state().boundary.run_id
            || receipt.owner_id != self.configuration.owner_id
            || receipt.actor_subject != actor_subject
            || receipt.source_id != active.source_id
            || receipt.source_version != active.version
            || receipt.source_digest != active.digest
            || receipt.expires_at <= now
            || receipt.binding.owner_id != self.configuration.owner_id
            || receipt.binding.workflow_run_id != receipt.workflow_run_id
            || link.source_id != active.source_id
            || link.source_version != active.version
            || link.source_digest != active.digest
            || link.active_revision_id != active.active_revision_id
            || link.publication_receipt_envelope_digest != publication.receipt_envelope_digest
        {
            return Err(ManagementError::conflict(
                "context_publication_active_stale",
                "active publication no longer matches authenticated owner evidence",
            ));
        }
        if validate_document(&source.document)? != active.digest {
            return Err(publication_active_conflict());
        }
        Ok((publication, source))
    }
}

pub(super) fn is_publication_source(source_id: &str) -> bool {
    source_id.starts_with("ownerpub.")
}

pub(super) fn publication_binding_matches(
    published: &ContextOwnerBinding,
    current: &ContextOwnerBinding,
) -> bool {
    published.owner_id == current.owner_id
        && published.owner_version == current.owner_version
        && published.binding_id == current.binding_id
        && published.binding_version == current.binding_version
        && published.binding_digest == current.binding_digest
        && published.context_ref == current.context_ref
        && published.instance_id == current.instance_id
        && published.node_kind == current.node_kind
        && published.state == current.state
        && published.workflow_run_id == current.workflow_run_id
        && published.definition_digest == current.definition_digest
        && published.graph_id == current.graph_id
        && published.node_id == current.node_id
        && published.node_execution_id == current.node_execution_id
        && published.lease_epoch == current.lease_epoch
        && published.snapshot_id == current.snapshot_id
        && published.grants == current.grants
        && published.continuity == current.continuity
}

pub(super) fn publication_store_error(
    error: sts2_harness::context_control::DurableControlStoreError,
) -> ManagementError {
    use sts2_harness::context_control::DurableControlStoreError as StoreError;
    match error {
        StoreError::PublicationConflict => ManagementError::conflict(
            "context_publication_request_id_reused",
            "publication request identity was already used with different content",
        ),
        StoreError::PublicationCapacity => ManagementError::unavailable(
            "context_publication_capacity",
            "owner publication capacity is exhausted",
        ),
        StoreError::PublicationMissing | StoreError::Missing => publication_missing(),
        StoreError::OwnerContextConflict => ManagementError::conflict(
            "context_publication_owner_state_stale",
            "owner state changed before publication could be committed",
        ),
        StoreError::ActivePublicationConflict | StoreError::SourceConflict => {
            publication_active_conflict()
        }
        _ => ManagementError::unavailable("context_publication_store", error.to_string()),
    }
}

pub(super) fn publication_unavailable() -> ManagementError {
    ManagementError::unavailable(
        "context_owner_publication_unavailable",
        "managed context publication is not enabled for this owner",
    )
}

pub(super) fn publication_missing() -> ManagementError {
    ManagementError::invalid(
        "context_publication_not_found",
        "owner publication is unavailable for this workflow run",
    )
}

pub(super) fn publication_active_conflict() -> ManagementError {
    ManagementError::conflict(
        "context_publication_active_stale",
        "active publication no longer matches the current owner revision",
    )
}
