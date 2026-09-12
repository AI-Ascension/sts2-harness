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
            compiler: result.compiler,
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

    /// Returns redacted context evidence for the current workflow cursor. This
    /// read does not construct provider input or grant control authority.
    pub fn context_association(
        &self,
        actor: &AuthContext,
        run_id: &str,
    ) -> Result<ContextAssociation, ManagementError> {
        validate_identifier("run_id", run_id)?;
        authorize(actor, "workflow:read", Some(run_id))?;
        let snapshot = self.store.get_run(run_id)?.ok_or_else(|| {
            ManagementError::invalid("run_not_found", "workflow run was not found")
        })?;
        let result = self.context_inspection.inspect(actor, &snapshot)?;
        validate_context_association_result(&result)?;
        Ok(ContextAssociation {
            schema_version: CONTEXT_ASSOCIATION_SCHEMA_VERSION.to_owned(),
            workflow: ContextWorkflowIdentity {
                workflow_run_id: snapshot.workflow_run_id,
                definition_digest: snapshot.definition_digest,
                graph_id: snapshot.cursor.graph_id,
                node_id: snapshot.cursor.node_id,
                node_execution_id: snapshot.cursor.node_execution_id,
            },
            context: result.context,
            capture: result.capture,
            capabilities: result.capabilities,
        })
    }
}

fn validate_context_association_result(
    result: &ContextInspectionResult,
) -> Result<(), ManagementError> {
    let context = &result.context;
    match context.availability {
        ContextAvailability::Available => {
            for (field, value) in [
                ("context_ref", context.context_ref.as_deref()),
                ("context_run_id", context.run_id.as_deref()),
                ("episode_id", context.episode_id.as_deref()),
                ("agent_id", context.agent_id.as_deref()),
                ("snapshot_id", context.snapshot_id.as_deref()),
                (
                    "approved_revision_id",
                    context.approved_revision_id.as_deref(),
                ),
            ] {
                validate_identifier(
                    field,
                    value.ok_or_else(|| {
                        ManagementError::invalid(
                            "context_association_incomplete",
                            "available context association is missing a required identity",
                        )
                    })?,
                )?;
            }
            if context.plan_epoch.unwrap_or_default() == 0 {
                return Err(ManagementError::invalid(
                    "context_association_incomplete",
                    "available context association is missing a positive plan epoch",
                ));
            }
        }
        ContextAvailability::Unavailable | ContextAvailability::NotApplicable => {
            if context.context_ref.is_some()
                || context.run_id.is_some()
                || context.episode_id.is_some()
                || context.agent_id.is_some()
                || context.snapshot_id.is_some()
                || context.approved_revision_id.is_some()
                || context.plan_epoch.is_some()
            {
                return Err(ManagementError::invalid(
                    "context_association_incomplete",
                    "unavailable context association must not invent identities",
                ));
            }
        }
    }
    if let Some(attempt_id) = &result.capture.attempt_id {
        validate_identifier("capture_attempt_id", attempt_id)?;
    }
    Ok(())
}
