// SPDX-License-Identifier: MIT

use super::support::{authorize, ensure_json_format, verify_digest};
use super::*;

impl ManagementService {
    pub fn validate(
        &self,
        actor: &AuthContext,
        request: ValidateRequest,
    ) -> Result<ValidateResponse, ManagementError> {
        authorize(actor, "workflow:read", None)?;
        schema_is(&request.schema_version, MANAGEMENT_SCHEMA_VERSION)?;
        let digest = digest_value(&request.definition)?;
        let result = self
            .definitions
            .validate(&request.definition, &request.capabilities)?;
        verify_digest(&digest, &result.definition_digest, "definition")?;
        Ok(ValidateResponse {
            schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
            valid: result.diagnostics.iter().all(|diagnostic| {
                diagnostic.severity != super::super::contract::DiagnosticSeverity::Error
            }),
            definition_digest: digest,
            diagnostics: result.diagnostics,
        })
    }

    pub fn inspect(
        &self,
        actor: &AuthContext,
        request: InspectRequest,
    ) -> Result<InspectResponse, ManagementError> {
        authorize(actor, "workflow:read", None)?;
        schema_is(&request.schema_version, MANAGEMENT_SCHEMA_VERSION)?;
        ensure_json_format(&request.format)?;
        let digest = digest_value(&request.definition)?;
        let result = self.definitions.inspect(&request.definition)?;
        verify_digest(&digest, &result.definition_digest, "definition")?;
        Ok(InspectResponse {
            schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
            definition_digest: digest,
            workflow_id: result.workflow_id,
            workflow_version: result.workflow_version,
            required_capabilities: result.required_capabilities,
            graph_count: result.graph_count,
            node_count: result.node_count,
        })
    }

    pub fn diff(
        &self,
        actor: &AuthContext,
        request: DiffRequest,
    ) -> Result<DiffResponse, ManagementError> {
        authorize(actor, "workflow:read", None)?;
        schema_is(&request.schema_version, MANAGEMENT_SCHEMA_VERSION)?;
        ensure_json_format(&request.format)?;
        let old_digest = digest_value(&request.old_definition)?;
        let new_digest = digest_value(&request.new_definition)?;
        let result = self
            .definitions
            .diff(&request.old_definition, &request.new_definition)?;
        verify_digest(&old_digest, &result.old_definition_digest, "old_definition")?;
        verify_digest(&new_digest, &result.new_definition_digest, "new_definition")?;
        Ok(DiffResponse {
            schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
            old_definition_digest: old_digest,
            new_definition_digest: new_digest,
            semantic_change: result.semantic_change,
            changed_paths: result.changed_paths,
        })
    }
}
