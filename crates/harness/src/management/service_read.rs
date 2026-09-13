// SPDX-License-Identifier: MIT

use super::support::{authorize, ensure_json_format, verify_digest};
use super::*;

impl ManagementService {
    /// Returns the target catalog already scoped by the authoritative target
    /// owner. The management service validates only its bounded, redacted
    /// shape; it never discovers instances or credentials itself.
    pub fn target_catalog(
        &self,
        actor: &AuthContext,
    ) -> Result<TargetCatalogResponse, ManagementError> {
        authorize(actor, "workflow:read", None)?;
        let catalog = self.capabilities.target_catalog(actor)?;
        catalog.validate().map_err(ManagementError::from)?;
        Ok(catalog)
    }

    /// Performs a read-only admission preflight against the current,
    /// actor-scoped catalog. No execution, provider, game, or lease effect is
    /// reachable from this method.
    pub fn preflight_target(
        &self,
        actor: &AuthContext,
        request: TargetAdmissionRequest,
    ) -> Result<TargetPreflightResponse, ManagementError> {
        authorize(actor, "workflow:control", None)?;
        request.validate().map_err(ManagementError::from)?;
        let catalog = self.target_catalog(actor)?;
        let descriptor = catalog
            .targets
            .iter()
            .find(|target| target.instance_id == request.target.instance_id)
            .ok_or_else(|| {
                ManagementError::unavailable(
                    "target_unavailable",
                    "requested target is not available to this actor",
                )
            })?;
        validate_target_selection(descriptor, &request.target)?;
        let binding = TargetAdmissionBinding {
            schema_version: TARGET_ADMISSION_SCHEMA_VERSION.to_owned(),
            request_id: request.request_id,
            workflow_definition_digest: request.workflow_definition_digest,
            target: request.target,
            descriptor_digest: descriptor.digest().map_err(ManagementError::from)?,
            catalog_revision: catalog.catalog_revision,
        };
        binding.validate().map_err(ManagementError::from)?;
        Ok(TargetPreflightResponse {
            schema_version: TARGET_ADMISSION_SCHEMA_VERSION.to_owned(),
            admission: binding,
        })
    }

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

    /// Returns a read-only, redacted provider-session projection for one
    /// workflow run. This is intentionally separate from context inspection:
    /// the adapter must establish any cross-owner binding explicitly.
    pub fn provider_sessions(
        &self,
        actor: &AuthContext,
        run_id: &str,
    ) -> Result<ProviderSessionListResponse, ManagementError> {
        validate_identifier("run_id", run_id)?;
        authorize(actor, "workflow:read", Some(run_id))?;
        let snapshot = self.store.get_run(run_id)?.ok_or_else(|| {
            ManagementError::invalid("run_not_found", "workflow run was not found")
        })?;
        let result = self.provider_session_inspection.list(actor, &snapshot)?;
        validate_provider_session_inspection_result(&result, &snapshot)?;
        Ok(ProviderSessionListResponse {
            schema: PROVIDER_SESSION_LIST_SCHEMA_VERSION.to_owned(),
            operation: "list".to_owned(),
            value: ProviderSessionListValue {
                run_id: snapshot.workflow_run_id,
                bindings: result.bindings,
                operations: result.operations,
                next_cursor: result.next_cursor,
            },
            effect_class: "local_metadata_only".to_owned(),
            inference_calls: 0,
            game_effects: 0,
        })
    }
}

fn validate_target_selection(
    descriptor: &TargetDescriptor,
    selection: &RunTargetConfiguration,
) -> Result<(), ManagementError> {
    match descriptor.availability {
        TargetAvailability::Available => {}
        TargetAvailability::Unavailable => {
            return Err(ManagementError::unavailable(
                "target_unavailable",
                "requested target is currently unavailable",
            ));
        }
        TargetAvailability::Revoked => {
            return Err(ManagementError::forbidden(
                "target_revoked",
                "requested target admission has been revoked",
            ));
        }
        TargetAvailability::Expired => {
            return Err(ManagementError::conflict(
                "target_expired",
                "requested target admission has expired",
            ));
        }
    }
    if descriptor.execution_mode != selection.execution_mode {
        return Err(ManagementError::conflict(
            "target_mode_mismatch",
            "target admission execution mode does not match the target",
        ));
    }
    if !descriptor
        .execution_profiles
        .iter()
        .any(|profile| profile == &selection.execution_profile)
    {
        return Err(ManagementError::capability(
            "target_profile_unavailable",
            "requested execution profile is not available on the target",
        ));
    }
    if descriptor.compatibility_revision != selection.compatibility_revision {
        return Err(ManagementError::conflict(
            "target_compatibility_stale",
            "target compatibility revision is stale",
        ));
    }
    if descriptor.capability_revision != selection.capability_revision {
        return Err(ManagementError::conflict(
            "target_capability_stale",
            "target capability revision is stale",
        ));
    }
    validate_target_profile(
        "game",
        &selection.game_profile,
        &descriptor.game_profiles,
        "target_game_profile_unavailable",
    )?;
    validate_optional_target_profile(
        "save",
        selection.save_profile.as_deref(),
        &descriptor.save_profiles,
        "target_save_profile_unavailable",
    )?;
    validate_optional_target_profile(
        "inference",
        selection.inference_profile.as_deref(),
        &descriptor.inference_profiles,
        "target_inference_profile_unavailable",
    )?;
    validate_optional_capability(
        "context",
        selection.context_capability.as_deref(),
        &descriptor.capabilities,
        "target_context_capability_unavailable",
    )?;
    validate_optional_capability(
        "provider",
        selection.provider_capability.as_deref(),
        &descriptor.capabilities,
        "target_provider_capability_unavailable",
    )?;
    Ok(())
}

fn validate_target_profile(
    namespace: &str,
    requested: &str,
    supported: &[String],
    code: &str,
) -> Result<(), ManagementError> {
    if supported.iter().any(|value| value == requested) {
        return Ok(());
    }
    Err(ManagementError::capability(
        code,
        format!("requested {namespace} profile is not available on the target"),
    ))
}

fn validate_optional_target_profile(
    namespace: &str,
    requested: Option<&str>,
    supported: &[String],
    code: &str,
) -> Result<(), ManagementError> {
    if let Some(requested) = requested {
        validate_target_profile(namespace, requested, supported, code)?;
    }
    Ok(())
}

fn validate_optional_capability(
    namespace: &str,
    requested: Option<&str>,
    supported: &[String],
    code: &str,
) -> Result<(), ManagementError> {
    if let Some(requested) = requested
        && !supported.iter().any(|value| value == requested)
    {
        return Err(ManagementError::capability(
            code,
            format!("requested {namespace} capability is not available on the target"),
        ));
    }
    Ok(())
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

fn validate_provider_session_inspection_result(
    result: &ProviderSessionInspectionResult,
    snapshot: &RunSnapshot,
) -> Result<(), ManagementError> {
    if result.workflow_run_id != snapshot.workflow_run_id {
        return Err(ManagementError::conflict(
            "provider_session_run_mismatch",
            "provider-session adapter returned a different workflow run identity",
        ));
    }
    if result.bindings.len() > 128 || result.operations.len() > 128 {
        return Err(ManagementError::invalid(
            "provider_session_projection_capacity",
            "provider-session adapter exceeded the bounded management projection",
        ));
    }
    for binding in &result.bindings {
        validate_identifier("provider_session_binding_id", &binding.binding_id)?;
        validate_identifier("provider_session_binding_state", &binding.state)?;
        validate_identifier(
            "provider_session_history_coverage",
            &binding.history_coverage,
        )?;
    }
    for operation in &result.operations {
        validate_identifier("provider_session_operation_id", &operation.operation_id)?;
        validate_identifier("provider_session_operation_state", &operation.state)?;
        if operation.game_effects != 0 || operation.auto_resume {
            return Err(ManagementError::invalid(
                "provider_session_projection_effectful",
                "provider-session inspection must remain metadata-only",
            ));
        }
    }
    if let Some(cursor) = &result.next_cursor {
        validate_identifier("provider_session_next_cursor", cursor)?;
    }
    Ok(())
}
