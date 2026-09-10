// SPDX-License-Identifier: MIT

use super::support::{
    authorize, enforce_live_profile, run_submission_response, validate_profile, verify_admission,
    verify_digest, waiting_reason,
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
        let admission = self.execution.submit(&request, actor, &definition_digest)?;
        verify_admission(&admission, &definition_digest)?;
        self.store.create_run(
            &request.request_id,
            &request_digest,
            admission.snapshot.clone(),
            admission.initial_events,
        )?;
        Ok(run_submission_response(&admission.snapshot))
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
        let application = self.execution.apply_command(CommandContext {
            request: request.clone(),
            snapshot: snapshot.clone(),
            actor: actor.clone(),
        })?;
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

    pub fn replay(
        &self,
        actor: &AuthContext,
        request: ReplayRequest,
    ) -> Result<ReplayResponse, ManagementError> {
        schema_is(&request.schema_version, MANAGEMENT_SCHEMA_VERSION)?;
        validate_identifier("run_id", &request.run_id)?;
        authorize(actor, "workflow:read", Some(&request.run_id))?;
        if !request.offline {
            return Err(ManagementError::invalid(
                "offline_required",
                "replay requires offline=true",
            ));
        }
        let snapshot = self.store.get_run(&request.run_id)?.ok_or_else(|| {
            ManagementError::invalid("run_not_found", "workflow run was not found")
        })?;
        let page = self.store.events(
            &request.run_id,
            0,
            super::super::contract::MAX_EVENTS_PER_PAGE,
        )?;
        let mut events = page.events;
        let mut cursor = page.next_after_sequence;
        while let Some(newest) = page.newest_sequence {
            if cursor >= newest {
                break;
            }
            let next = self.store.events(
                &request.run_id,
                cursor,
                super::super::contract::MAX_EVENTS_PER_PAGE,
            )?;
            if next.events.is_empty() {
                break;
            }
            cursor = next.next_after_sequence;
            events.extend(next.events);
        }
        let result = self.replay.replay(&request, &snapshot, &events)?;
        if !result.matched {
            return Err(ManagementError::replay(
                "replay_divergence",
                result
                    .first_divergence
                    .as_ref()
                    .map(|divergence| format!("replay diverged at {}", divergence.path))
                    .unwrap_or_else(|| "replay diverged".to_owned()),
            ));
        }
        Ok(ReplayResponse {
            schema_version: REPLAY_SCHEMA_VERSION.to_owned(),
            workflow_run_id: request.run_id,
            matched: true,
            compared_events: result.compared_events,
            first_divergence: None,
        })
    }

    pub fn export(
        &self,
        actor: &AuthContext,
        request: ExportRequest,
    ) -> Result<ExportResponse, ManagementError> {
        schema_is(&request.schema_version, MANAGEMENT_SCHEMA_VERSION)?;
        validate_identifier("run_id", &request.run_id)?;
        authorize(actor, "workflow:export", Some(&request.run_id))?;
        if !request.redacted {
            return Err(ManagementError::forbidden(
                "redaction_required",
                "management export must be redacted",
            ));
        }
        Ok(self.store.export(&request.run_id, true)?)
    }

    pub fn capabilities(&self, actor: &AuthContext) -> Result<CapabilityResponse, ManagementError> {
        authorize(actor, "workflow:read", None)?;
        Ok(CapabilityResponse {
            schema_version: super::super::contract::CAPABILITIES_SCHEMA_VERSION.to_owned(),
            capabilities: self.capabilities.capabilities()?,
        })
    }

    pub fn health(&self) -> HealthResponse {
        HealthResponse {
            schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
            status: "ok".to_owned(),
        }
    }
}
