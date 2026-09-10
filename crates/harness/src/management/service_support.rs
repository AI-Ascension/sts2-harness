// SPDX-License-Identifier: MIT

use super::*;

pub(super) fn authorize(
    actor: &AuthContext,
    scope: &str,
    run_id: Option<&str>,
) -> Result<(), ManagementError> {
    if !actor.can(scope) {
        return Err(ManagementError::forbidden(
            "missing_scope",
            "authenticated actor lacks the required management scope",
        ));
    }
    if let Some(run_id) = run_id
        && !actor.can_run(run_id)
    {
        return Err(ManagementError::forbidden(
            "run_scope_denied",
            "authenticated actor is not authorized for this workflow run",
        ));
    }
    Ok(())
}

pub(super) fn ensure_json_format(format: &OutputFormat) -> Result<(), ManagementError> {
    if !matches!(format, OutputFormat::Json) {
        return Err(ManagementError::invalid(
            "unsupported_format",
            "only JSON output is supported",
        ));
    }
    Ok(())
}

pub(super) fn validate_profile(profile: &str) -> Result<(), ManagementError> {
    validate_identifier("profile", profile)?;
    if profile == "synthetic"
        || profile.starts_with("synthetic.")
        || profile == "live"
        || profile.starts_with("live.")
    {
        Ok(())
    } else {
        Err(ManagementError::invalid(
            "profile_must_be_explicit",
            "profile must be a registered synthetic.* or live.* profile",
        ))
    }
}

pub(super) fn enforce_live_profile(
    capabilities: &Arc<dyn CapabilityPort>,
    actor: &AuthContext,
    profile: &str,
) -> Result<(), ManagementError> {
    if !(profile == "live" || profile.starts_with("live.")) {
        return Ok(());
    }
    if !actor.can("workflow:live") {
        return Err(ManagementError::forbidden(
            "live_scope_required",
            "live workflow execution requires the workflow:live scope",
        ));
    }
    let value = capabilities.capabilities()?;
    if !has_capability(&value, "workflow.live") && !has_capability(&value, profile) {
        return Err(ManagementError::capability(
            "live_capability_unavailable",
            "the requested live workflow profile is unavailable",
        ));
    }
    Ok(())
}

fn has_capability(value: &Value, requested: &str) -> bool {
    value
        .get("capabilities")
        .and_then(Value::as_array)
        .is_some_and(|values| values.iter().any(|value| value.as_str() == Some(requested)))
}

pub(super) fn verify_digest(
    expected: &str,
    actual: &str,
    field: &str,
) -> Result<(), ManagementError> {
    validate_digest(field, actual)?;
    if expected != actual {
        return Err(ManagementError::conflict(
            "port_digest_mismatch",
            "injected port returned a digest different from the request",
        ));
    }
    Ok(())
}

pub(super) fn verify_admission(
    admission: &RunAdmission,
    definition_digest: &str,
) -> Result<(), ManagementError> {
    let snapshot = &admission.snapshot;
    schema_is(&snapshot.schema_version, RUN_SCHEMA_VERSION)?;
    validate_identifier("workflow_run_id", &snapshot.workflow_run_id)?;
    validate_digest("definition_digest", &snapshot.definition_digest)?;
    if snapshot.definition_digest != definition_digest {
        return Err(ManagementError::conflict(
            "port_digest_mismatch",
            "execution port returned a different definition digest",
        ));
    }
    if snapshot.run_revision == 0 || admission.initial_events.is_empty() {
        return Err(ManagementError::invalid(
            "invalid_admission",
            "execution port returned an incomplete durable admission",
        ));
    }
    for (index, event) in admission.initial_events.iter().enumerate() {
        let expected_sequence = (index as u64).saturating_add(1);
        if event.sequence != expected_sequence
            || event.workflow_run_id != snapshot.workflow_run_id
            || event.definition_digest != snapshot.definition_digest
        {
            return Err(ManagementError::invalid(
                "invalid_admission_events",
                "execution port returned non-contiguous admission events",
            ));
        }
    }
    Ok(())
}

pub(super) fn run_submission_response(snapshot: &RunSnapshot) -> RunSubmissionResponse {
    RunSubmissionResponse {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        workflow_run_id: snapshot.workflow_run_id.clone(),
        run_revision: snapshot.run_revision,
        status: snapshot.status.clone(),
    }
}

pub(super) fn waiting_reason(status: &WorkflowRunStatus) -> Option<String> {
    match status {
        WorkflowRunStatus::WaitingForProvider => Some("provider".to_owned()),
        WorkflowRunStatus::WaitingForGame => Some("game".to_owned()),
        WorkflowRunStatus::Pausing => Some("safe_point".to_owned()),
        WorkflowRunStatus::Reconciling => Some("effect_reconciliation".to_owned()),
        WorkflowRunStatus::NeedsOperator => Some("operator".to_owned()),
        _ => None,
    }
}

pub struct UnavailableDefinitionPort;

impl DefinitionPort for UnavailableDefinitionPort {
    fn validate(
        &self,
        _definition: &Value,
        _capabilities: &Value,
    ) -> Result<ValidationResult, ManagementError> {
        Err(ManagementError::unavailable(
            "definition_port_unavailable",
            "workflow definition compiler/validator is not injected",
        ))
    }

    fn inspect(&self, _definition: &Value) -> Result<InspectionResult, ManagementError> {
        Err(ManagementError::unavailable(
            "definition_port_unavailable",
            "workflow definition compiler/validator is not injected",
        ))
    }

    fn diff(
        &self,
        _old_definition: &Value,
        _new_definition: &Value,
    ) -> Result<DiffResult, ManagementError> {
        Err(ManagementError::unavailable(
            "definition_port_unavailable",
            "workflow definition compiler/validator is not injected",
        ))
    }
}

pub struct UnavailableExecutionPort;

impl WorkflowExecutionPort for UnavailableExecutionPort {
    fn submit(
        &self,
        _request: &RunRequest,
        _actor: &AuthContext,
        _definition_digest: &str,
    ) -> Result<RunAdmission, ManagementError> {
        Err(ManagementError::unavailable(
            "execution_port_unavailable",
            "workflow execution authority is not injected",
        ))
    }

    fn apply_command(
        &self,
        _context: CommandContext,
    ) -> Result<CommandApplication, ManagementError> {
        Err(ManagementError::unavailable(
            "execution_port_unavailable",
            "workflow execution authority is not injected",
        ))
    }
}

pub struct UnavailableReplayPort;

impl WorkflowReplayPort for UnavailableReplayPort {
    fn replay(
        &self,
        _request: &ReplayRequest,
        _snapshot: &RunSnapshot,
        _events: &[RunEvent],
    ) -> Result<ReplayResult, ManagementError> {
        Err(ManagementError::unavailable(
            "replay_port_unavailable",
            "offline replay reducer is not injected",
        ))
    }
}

pub struct UnavailableCapabilityPort;

impl CapabilityPort for UnavailableCapabilityPort {
    fn capabilities(&self) -> Result<Value, ManagementError> {
        Err(ManagementError::unavailable(
            "capability_port_unavailable",
            "capability authority is not injected",
        ))
    }
}
