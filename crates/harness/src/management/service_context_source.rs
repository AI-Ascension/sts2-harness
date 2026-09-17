// SPDX-License-Identifier: MIT

use super::super::super::context_owner::{
    CONTEXT_OWNER_SOURCE_STATUS_SCHEMA_VERSION, CONTEXT_SOURCE_ADOPTION_SCHEMA_VERSION,
    CONTEXT_SOURCE_UPLOAD_SCHEMA_VERSION, ContextOwnerSourceStatus, ContextSourceAdoptionRequest,
    ContextSourcePublication, ContextSourceUpload,
};
use super::*;
use crate::context_control::context_source_digest;

impl ManagementService {
    pub fn current_context_source_status(
        &self,
        actor: &AuthContext,
        run_id: &str,
    ) -> Result<ContextOwnerSourceStatus, ManagementError> {
        validate_identifier("run_id", run_id)?;
        authorize(actor, "workflow:read", Some(run_id))?;
        let snapshot = self.store.get_run(run_id)?.ok_or_else(|| {
            ManagementError::invalid("run_not_found", "workflow run was not found")
        })?;
        let status = self.context_owner.source_status(actor, &snapshot)?;
        if status.schema_version != CONTEXT_OWNER_SOURCE_STATUS_SCHEMA_VERSION
            || status.workflow_run_id != snapshot.workflow_run_id
            || status.definition_digest != snapshot.definition_digest
            || snapshot
                .admission
                .as_ref()
                .is_some_and(|admission| admission.target.instance_id != status.instance_id)
        {
            return Err(ManagementError::conflict(
                "context_owner_source_status_scope",
                "source status does not match the admitted workflow target and run",
            ));
        }
        Ok(status)
    }

    /// Publishes bytes for an owner-advertised immutable source identity. The
    /// owner derives the content digest and encrypts the snapshot at rest.
    pub fn publish_context_source(
        &self,
        actor: &AuthContext,
        run_id: &str,
        source_id: &str,
        upload: ContextSourceUpload,
    ) -> Result<ContextSourcePublication, ManagementError> {
        validate_identifier("run_id", run_id)?;
        validate_identifier("context_source_id", source_id)?;
        authorize(actor, "workflow:content:write", Some(run_id))?;
        if upload.schema_version != CONTEXT_SOURCE_UPLOAD_SCHEMA_VERSION {
            return Err(ManagementError::invalid(
                "context_source_upload_schema",
                "context source upload schema is unsupported",
            ));
        }
        let snapshot = self.store.get_run(run_id)?.ok_or_else(|| {
            ManagementError::invalid("run_not_found", "workflow run was not found")
        })?;
        let expected_digest = context_source_digest(&upload.document).map_err(|error| {
            ManagementError::invalid("context_source_invalid", error.to_string())
        })?;
        let catalog = self.context_owner.catalog(actor)?;
        catalog.validate()?;
        if !catalog
            .descriptors
            .iter()
            .flat_map(|descriptor| descriptor.sources.iter())
            .any(|source| source.source_id == source_id && source.digest == expected_digest)
        {
            return Err(ManagementError::conflict(
                "context_source_not_advertised",
                "source identity and content digest are not advertised by this owner",
            ));
        }
        let source =
            self.context_owner
                .publish_source(actor, &snapshot, source_id, &upload.document)?;
        if source.source_id != source_id || source.digest != expected_digest {
            return Err(ManagementError::conflict(
                "context_source_publication_mismatch",
                "owner publication does not match the advertised source identity",
            ));
        }
        Ok(ContextSourcePublication {
            schema_version: CONTEXT_SOURCE_UPLOAD_SCHEMA_VERSION.to_owned(),
            source,
        })
    }

    /// Explicitly adopts a previously published source as one control revision.
    /// The owner derives both manifest digests from the encrypted source bytes.
    pub fn adopt_context_source(
        &self,
        actor: &AuthContext,
        run_id: &str,
        source_id: &str,
        request: ContextSourceAdoptionRequest,
    ) -> Result<ContextControlReceipt, ManagementError> {
        validate_identifier("run_id", run_id)?;
        validate_identifier("context_source_id", source_id)?;
        authorize(actor, "workflow:control", Some(run_id))?;
        if request.schema_version != CONTEXT_SOURCE_ADOPTION_SCHEMA_VERSION {
            return Err(ManagementError::invalid(
                "context_source_adoption_schema",
                "context source adoption schema is unsupported",
            ));
        }
        let snapshot = self.store.get_run(run_id)?.ok_or_else(|| {
            ManagementError::invalid("run_not_found", "workflow run was not found")
        })?;
        let catalog = self.context_owner.catalog(actor)?;
        catalog.validate()?;
        if !catalog
            .descriptors
            .iter()
            .flat_map(|descriptor| descriptor.sources.iter())
            .any(|source| source.source_id == source_id)
        {
            return Err(ManagementError::capability(
                "context_source_not_advertised",
                "source identity is not advertised by this owner",
            ));
        }
        self.context_owner
            .adopt_source(actor, &snapshot, source_id, &request)
    }
}
