// SPDX-License-Identifier: MIT

use super::submission;
use super::support::{
    authorize, enforce_live_profile, recovery_admission, run_submission_response, validate_profile,
    verify_digest, waiting_reason,
};
use super::target_admission::{
    bind_snapshot_admission, is_live_profile, validate_run_target_admission,
};
use super::*;

impl ManagementService {
    pub fn submit_run(
        &self,
        actor: &AuthContext,
        request: RunRequest,
    ) -> Result<RunSubmissionResponse, ManagementError> {
        authorize(actor, "workflow:control", None)?;
        schema_is(&request.schema_version, MANAGEMENT_SCHEMA_VERSION)?;
        validate_identifier("request_id", &request.request_id)?;
        validate_identifier("instance_id", &request.instance_id)?;
        validate_profile(&request.profile)?;
        validate_run_target_admission(&request)?;
        if request.definition.is_some() == request.artifact_id.is_some() {
            return Err(ManagementError::invalid(
                "run_source_count",
                "exactly one definition or artifact_id is required",
            ));
        }
        if let Some(artifact_id) = &request.artifact_id {
            validate_identifier("artifact_id", artifact_id)?;
        }
        enforce_live_profile(&self.capabilities, actor, &request.profile)?;
        let request_value = serde_json::to_value(&request)
            .map_err(|error| ManagementError::invalid("run_request_encode", error.to_string()))?;
        let request_digest = digest_value(&request_value)?;
        match self
            .store
            .lookup_submission(&request.request_id, &request_digest)?
        {
            SubmissionLookup::Existing(snapshot) => {
                if is_live_profile(&request.profile) {
                    let recovery = self
                        .execution
                        .recovery_admission(&snapshot)
                        .unwrap_or_else(|| recovery_admission(&snapshot));
                    if matches!(
                        recovery,
                        RecoveryAdmission::NeedsOperator | RecoveryAdmission::Reconciling
                    ) {
                        return Err(ManagementError::unresolved(
                            "live_submission_recovery_required",
                            "a reserved live submission requires reconciliation before retry",
                        ));
                    }
                }
                return Ok(run_submission_response(&snapshot));
            }
            SubmissionLookup::Conflict => {
                return Err(ManagementError::conflict(
                    "submission_conflict",
                    "request_id was reused with a different submission",
                ));
            }
            SubmissionLookup::Missing => {}
        }
        let definition_digest = if let Some(definition) = &request.definition {
            let capabilities = match self.capabilities.capabilities() {
                Ok(value) => value,
                Err(error) if error.class == ErrorClass::Unavailable => {
                    json!({ "capabilities": [] })
                }
                Err(error) => return Err(error),
            };
            let validation = self.definitions.validate(definition, &capabilities)?;
            if validation.diagnostics.iter().any(|diagnostic| {
                diagnostic.severity == super::super::contract::DiagnosticSeverity::Error
            }) {
                return Err(ManagementError::invalid(
                    "definition_invalid",
                    "workflow definition validation returned an error diagnostic",
                ));
            }
            let digest = digest_value(definition)?;
            verify_digest(&digest, &validation.definition_digest, "definition")?;
            digest
        } else {
            digest_value(&json!({ "artifact_id": request.artifact_id }))?
        };
        submission::submit_run(self, actor, request, request_digest, definition_digest)
    }

    pub fn status(
        &self,
        actor: &AuthContext,
        run_id: &str,
    ) -> Result<StatusResponse, ManagementError> {
        validate_identifier("run_id", run_id)?;
        authorize(actor, "workflow:read", Some(run_id))?;
        let snapshot = self.store.get_run(run_id)?.ok_or_else(|| {
            ManagementError::invalid("run_not_found", "workflow run was not found")
        })?;
        let recovery = self
            .execution
            .recovery_admission(&snapshot)
            .unwrap_or_else(|| recovery_admission(&snapshot));
        Ok(StatusResponse {
            schema_version: STATUS_SCHEMA_VERSION.to_owned(),
            accepted_plan_revision: None,
            waiting_reason: waiting_reason(&snapshot.status),
            authority: AuthoritySummary {
                state: "engine_port_owned".to_owned(),
                recovery: if snapshot.pending_operation.is_some() {
                    "pending_effect_visible".to_owned()
                } else {
                    "none".to_owned()
                },
            },
            recovery_admission: recovery,
            last_progress_sequence: snapshot.run_revision,
            run: snapshot,
        })
    }

    pub fn events(
        &self,
        actor: &AuthContext,
        run_id: &str,
        after_sequence: u64,
        limit: u64,
    ) -> Result<EventPage, ManagementError> {
        validate_identifier("run_id", run_id)?;
        authorize(actor, "workflow:read", Some(run_id))?;
        if limit == 0 || limit > super::super::contract::MAX_EVENTS_PER_PAGE {
            return Err(ManagementError::invalid(
                "event_limit",
                "event page limit is outside the supported bound",
            ));
        }
        Ok(self.store.events(run_id, after_sequence, limit)?)
    }

    pub fn command(
        &self,
        actor: &AuthContext,
        request: CommandRequest,
    ) -> Result<CommandResponse, ManagementError> {
        schema_is(&request.schema_version, MANAGEMENT_SCHEMA_VERSION)?;
        validate_identifier("command_id", &request.command_id)?;
        validate_identifier("run_id", &request.run_id)?;
        validate_identifier("actor_scope", &request.actor_scope)?;
        if request.expected_revision == 0 {
            return Err(ManagementError::invalid(
                "invalid_revision",
                "expected_revision must be positive",
            ));
        }
        if request.actor_scope != actor.subject {
            return Err(ManagementError::forbidden(
                "actor_scope_mismatch",
                "command actor scope does not match authenticated identity",
            ));
        }
        authorize(actor, "workflow:control", Some(&request.run_id))?;
        let request_value = serde_json::to_value(&request)
            .map_err(|error| ManagementError::invalid("command_encode", error.to_string()))?;
        let request_digest = digest_value(&request_value)?;
        let acceptance = self.store.accept_command(&request, &request_digest)?;
        let snapshot = match acceptance {
            CommandAcceptance::Conflict => {
                return Err(ManagementError::conflict(
                    "command_conflict",
                    "command ID was reused with a different payload",
                ));
            }
            CommandAcceptance::Existing {
                snapshot: _,
                response: Some(response),
                application_in_flight: _,
            } => return Ok(response),
            CommandAcceptance::Existing {
                snapshot,
                response: None,
                application_in_flight: true,
            } => {
                return Ok(CommandResponse {
                    schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
                    command_id: request.command_id,
                    workflow_run_id: request.run_id,
                    outcome: super::super::contract::CommandOutcome::Pending,
                    run_revision: snapshot.run_revision,
                    sequence: None,
                });
            }
            CommandAcceptance::Existing {
                snapshot,
                response: None,
                application_in_flight: false,
            }
            | CommandAcceptance::New { snapshot, .. } => snapshot,
        };
        if snapshot.run_revision != request.expected_revision {
            return Err(ManagementError::conflict(
                "stale_revision",
                "command revision is stale after acceptance",
            ));
        }
        let store = Arc::clone(&self.store);
        let run_id = request.run_id.clone();
        let expected_revision = request.expected_revision;
        let record_intent = move |pending| {
            store
                .record_operation_intent(&run_id, expected_revision, pending)
                .map_err(ManagementError::from)
        };
        let mut application = match self.execution.apply_command_with_intent(
            CommandContext {
                request: request.clone(),
                snapshot: snapshot.clone(),
                actor: actor.clone(),
            },
            &record_intent,
        ) {
            Ok(application) => application,
            Err(error) => {
                if error.class != ErrorClass::Unresolved {
                    let _ = self.store.release_command(&request, &request_digest);
                }
                return Err(error);
            }
        };
        application.snapshot =
            bind_snapshot_admission(application.snapshot, snapshot.admission.as_ref())?;
        let response = self.store.apply_command(
            &request,
            &request_digest,
            StoredCommandApplication {
                snapshot: application.snapshot,
                outcome: application.outcome,
                reason_code: application.reason_code,
            },
        )?;
        Ok(response)
    }
}
