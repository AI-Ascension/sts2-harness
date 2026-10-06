// SPDX-License-Identifier: MIT

use super::drafts::{owner_lock_error, owner_unavailable};
use super::publication_active::{
    is_publication_source, publication_active_conflict, publication_missing,
    publication_store_error,
};
use super::source::{unix_time, validate_document};
use super::*;
use sts2_harness::context_control::DurableActiveContextSource;
use sts2_harness::management::ContextSourceAdoptionRequest;

impl Owner {
    pub(super) fn adopt_published_source_current(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        source_id: &str,
        request: &ContextSourceAdoptionRequest,
    ) -> Result<sts2_harness::management::ContextControlReceipt, ManagementError> {
        if !self.configuration.render_required
            || !actor.can("workflow:control")
            || !actor.can_run(&snapshot.workflow_run_id)
            || request.schema_version
                != sts2_harness::management::CONTEXT_SOURCE_ADOPTION_SCHEMA_VERSION
        {
            return Err(ManagementError::forbidden(
                "context_source_adoption_forbidden",
                "actor cannot adopt an owner publication for this workflow run",
            ));
        }
        if !is_publication_source(source_id) {
            return Err(publication_missing());
        }
        let run_id = self.validate_snapshot_owner(actor, snapshot)?;
        let mut current = self.current.lock().map_err(|_| owner_lock_error())?;
        let entry = current.get_mut(&run_id).ok_or_else(owner_unavailable)?;
        let publication = entry
            .store
            .list_draft_publications(&self.configuration.owner_id, &actor.subject)
            .map_err(publication_store_error)?
            .into_iter()
            .find(|publication| publication.receipt.source_id == source_id)
            .ok_or_else(publication_missing)?;
        let advertised = ContextBindingSource {
            source_id: publication.receipt.source_id.clone(),
            version: publication.receipt.source_version,
            digest: publication.receipt.source_digest.clone(),
        };
        let command = sts2_harness::management::ContextControlCommand::Commit {
            idempotency_key: request.idempotency_key.clone(),
            expected_control_version: request.expected_control_version,
            expected_revision_id: request.expected_revision_id.clone(),
            expected_boundary: request.expected_boundary.clone(),
            preview_manifest_digest: advertised.digest.clone(),
            approved_manifest_digest: advertised.digest.clone(),
        };
        // Control receipt replay precedes current boundary, expiry, binding, and revision checks.
        if let Some(existing) = entry
            .store
            .lookup_owner_control_receipt(&self.configuration.owner_id, &actor.subject, &command)
            .map_err(publication_store_error)?
        {
            existing.receipt.validate_for(&existing.binding, &command)?;
            let adoption_receipt = entry
                .authority
                .lookup_source_adoption_receipt(
                    &request.idempotency_key,
                    request.expected_control_version,
                    &request.expected_revision_id,
                    &request.expected_boundary,
                    &advertised.source_id,
                    advertised.version,
                    &advertised.digest,
                )
                .map_err(|error| {
                    ManagementError::conflict("context_source_adoption_refused", error)
                })?;
            validate_adoption_replay_receipt(&existing.receipt, adoption_receipt.as_ref())?;
            return Ok(existing.receipt);
        }
        let now = unix_time()?;
        let state = entry.authority.state().clone();
        if entry.catalog_generation != Some(state.boundary.generation)
            || request.expected_revision_id != state.active_revision_id
            || request.expected_control_version != state.boundary.control_version
            || request.expected_boundary != state.boundary
            || publication.receipt.expires_at <= now
            || publication.receipt.base_revision_id != state.active_revision_id
            || publication.receipt.boundary != state.boundary
        {
            return Err(ManagementError::conflict(
                "context_source_adoption_stale",
                "publication preconditions do not match the current owner control fence",
            ));
        }
        let (saved_publication, source) = entry
            .store
            .load_publication_source(
                &self.configuration.owner_id,
                &actor.subject,
                &advertised.source_id,
                advertised.version,
                &advertised.digest,
            )
            .map_err(publication_store_error)?
            .ok_or_else(publication_missing)?;
        if saved_publication != publication
            || validate_document(&source.document)? != advertised.digest
            || source.document.draft.base_revision_id != state.active_revision_id
        {
            return Err(publication_active_conflict());
        }
        let binding = self.authorize_current_binding(actor, snapshot, entry, false)?;
        let descriptor = self
            .catalog(actor)?
            .descriptor_for(&self.configuration.context_ref, "decide")?;
        if !descriptor.grants.content_read
            || !binding.grants.content_read
            || publication.receipt.binding != binding
        {
            return Err(ManagementError::capability(
                "context_source_grant_missing",
                "current binding does not grant adoption of this owner publication",
            ));
        }
        self.validate_control_limits_in_catalog(
            &self.catalog(actor)?,
            &entry.admitted_control_limits,
        )?;
        let mut authority = entry.authority.clone();
        let outcome = authority
            .adopt_source_revision(
                &request.idempotency_key,
                request.expected_control_version,
                &request.expected_revision_id,
                &request.expected_boundary,
                &advertised.source_id,
                advertised.version,
                &advertised.digest,
            )
            .map_err(|error| ManagementError::conflict("context_source_adoption_refused", error))?;
        let active_state = authority.state();
        let receipt = sts2_harness::management::ContextControlReceipt {
            schema_version: sts2_harness::management::CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION
                .to_owned(),
            owner_id: self.configuration.owner_id.clone(),
            invocation_id: binding.invocation_id.clone(),
            binding_id: binding.binding_id.clone(),
            binding_digest: binding.binding_digest.clone(),
            command: sts2_harness::management::ContextControlCommandKind::Commit,
            command_id: outcome.command_id,
            idempotency_key: outcome.idempotency_key,
            effect: outcome.effect,
            control_version: outcome.control_version,
            plan_epoch: outcome.plan_epoch,
            controller_epoch: active_state.boundary.controller_epoch,
            gate_epoch: active_state.boundary.gate_epoch,
            boundary: active_state.boundary.clone(),
            revision_id: Some(active_state.active_revision_id.clone()),
            preview_manifest_digest: Some(advertised.digest.clone()),
            approved_manifest_digest: Some(advertised.digest.clone()),
        };
        receipt.validate_for(&binding, &command)?;
        let durable = sts2_harness::context_control::DurableContextOwnerControlReceipt {
            owner_id: self.configuration.owner_id.clone(),
            actor_subject: actor.subject.clone(),
            binding,
            command,
            receipt: receipt.clone(),
        };
        let activation = DurableActiveContextSource {
            source_id: advertised.source_id,
            version: advertised.version,
            digest: advertised.digest,
            active_revision_id: active_state.active_revision_id.clone(),
        };
        entry
            .store
            .persist_with_owner_control_receipt_and_publication(
                &authority,
                sts2_harness::context_control::StoreMode::Enabled,
                &durable,
                &activation,
                &publication,
            )
            .map_err(publication_store_error)?;
        entry.authority = authority;
        Ok(receipt)
    }
}

pub(super) fn validate_adoption_replay_receipt(
    outer: &sts2_harness::management::ContextControlReceipt,
    inner: Option<&sts2_harness::context_control::ControlReceipt>,
) -> Result<(), ManagementError> {
    let Some(inner) = inner else {
        return Err(adoption_receipt_corrupt());
    };
    if inner.command_id != outer.command_id
        || inner.idempotency_key != outer.idempotency_key
        || inner.effect != outer.effect
        || inner.control_version != outer.control_version
        || inner.plan_epoch != outer.plan_epoch
    {
        return Err(adoption_receipt_corrupt());
    }
    Ok(())
}

fn adoption_receipt_corrupt() -> ManagementError {
    ManagementError::unavailable(
        "context_source_adoption_receipt_corrupt",
        "stored source-adoption history does not match its durable control receipt",
    )
}
