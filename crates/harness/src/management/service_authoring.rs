// SPDX-License-Identifier: MIT

use serde_json::json;

use super::super::authoring::{self, PublishResult};
use super::super::contract::{
    DiagnosticSeverity, ErrorClass, digest_value, schema_is, validate_digest, validate_identifier,
};
use super::super::contract_authoring::{
    STUDIO_SCHEMA_VERSION, StudioCreateDraftRequest, StudioDefinitionsResponse, StudioDraftRecord,
    StudioPublishDraftRequest, StudioPublishResponse, StudioSaveDraftRequest,
};
use super::support::authorize;
use super::*;

impl ManagementService {
    pub fn studio_definitions(
        &self,
        actor: &AuthContext,
    ) -> Result<StudioDefinitionsResponse, ManagementError> {
        authorize(actor, "workflow:read", None)?;
        Ok(StudioDefinitionsResponse {
            schema_version: STUDIO_SCHEMA_VERSION.to_owned(),
            definitions: self.authoring.list_definitions()?,
        })
    }

    pub fn studio_draft(
        &self,
        actor: &AuthContext,
        draft_id: &str,
    ) -> Result<StudioDraftRecord, ManagementError> {
        authorize(actor, "workflow:read", None)?;
        validate_identifier("draft_id", draft_id)?;
        self.authoring.get_draft(draft_id)?.ok_or_else(|| {
            ManagementError::invalid("draft_not_found", "Studio draft was not found")
        })
    }

    pub fn studio_create_draft(
        &self,
        actor: &AuthContext,
        request: StudioCreateDraftRequest,
    ) -> Result<StudioDraftRecord, ManagementError> {
        authorize(actor, "workflow:author", None)?;
        schema_is(&request.schema_version, STUDIO_SCHEMA_VERSION)?;
        validate_identifier("draft_id", &request.draft_id)?;
        validate_identifier("definition_id", &request.definition_id)?;
        validate_identifier("client_mutation_id", &request.client_mutation_id)?;
        let draft = authoring::new_draft(
            &request.draft_id,
            &request.definition_id,
            request.document,
            request.layout,
        )?;
        Ok(self
            .authoring
            .create_draft(draft, &request.client_mutation_id)?)
    }

    pub fn studio_save_draft(
        &self,
        actor: &AuthContext,
        draft_id: &str,
        request: StudioSaveDraftRequest,
    ) -> Result<StudioDraftRecord, ManagementError> {
        authorize(actor, "workflow:author", None)?;
        schema_is(&request.schema_version, STUDIO_SCHEMA_VERSION)?;
        validate_identifier("draft_id", draft_id)?;
        validate_identifier("client_mutation_id", &request.client_mutation_id)?;
        validate_digest("etag", &request.etag)?;
        Ok(self.authoring.save_draft(
            draft_id,
            request.expected_revision,
            &request.etag,
            &request.client_mutation_id,
            request.document,
            request.layout,
        )?)
    }

    pub fn studio_publish_draft(
        &self,
        actor: &AuthContext,
        draft_id: &str,
        request: StudioPublishDraftRequest,
    ) -> Result<StudioPublishResponse, ManagementError> {
        authorize(actor, "workflow:publish", None)?;
        schema_is(&request.schema_version, STUDIO_SCHEMA_VERSION)?;
        validate_identifier("draft_id", draft_id)?;
        validate_identifier("client_mutation_id", &request.client_mutation_id)?;
        validate_digest("etag", &request.etag)?;
        validate_digest(
            "expected_definition_digest",
            &request.expected_definition_digest,
        )?;

        let draft = self.authoring.get_draft(draft_id)?.ok_or_else(|| {
            ManagementError::invalid("draft_not_found", "Studio draft was not found")
        })?;
        if draft.revision != request.expected_revision || draft.etag != request.etag {
            return Ok(StudioPublishResponse {
                schema_version: STUDIO_SCHEMA_VERSION.to_owned(),
                outcome: "conflict".to_owned(),
                definition: None,
                draft: Some(authoring::with_conflict(draft)),
            });
        }

        let capabilities = match self.capabilities.capabilities() {
            Ok(value) => value,
            Err(error) if error.class == ErrorClass::Unavailable => json!({"capabilities": []}),
            Err(error) => return Err(error),
        };
        let validation = self.definitions.validate(&draft.document, &capabilities)?;
        let digest = digest_value(&draft.document)?;
        super::support::verify_digest(&digest, &validation.definition_digest, "definition")?;
        if validation
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
        {
            return Err(ManagementError::invalid(
                "definition_invalid",
                "workflow definition validation returned an error diagnostic",
            ));
        }
        if digest != request.expected_definition_digest {
            return Ok(StudioPublishResponse {
                schema_version: STUDIO_SCHEMA_VERSION.to_owned(),
                outcome: "conflict".to_owned(),
                definition: None,
                draft: Some(authoring::with_conflict(draft)),
            });
        }

        let outcome = self.authoring.publish_draft(
            draft_id,
            request.expected_revision,
            &request.etag,
            &request.expected_definition_digest,
        )?;
        Ok(match outcome {
            PublishResult::Published(definition) => StudioPublishResponse {
                schema_version: STUDIO_SCHEMA_VERSION.to_owned(),
                outcome: "published".to_owned(),
                definition: Some(definition),
                draft: None,
            },
            PublishResult::AlreadyPublished(definition) => StudioPublishResponse {
                schema_version: STUDIO_SCHEMA_VERSION.to_owned(),
                outcome: "already_published".to_owned(),
                definition: Some(definition),
                draft: None,
            },
            PublishResult::Conflict(draft) => StudioPublishResponse {
                schema_version: STUDIO_SCHEMA_VERSION.to_owned(),
                outcome: "conflict".to_owned(),
                definition: None,
                draft: Some(draft),
            },
        })
    }
}
