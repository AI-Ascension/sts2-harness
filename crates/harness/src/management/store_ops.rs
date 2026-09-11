// SPDX-License-Identifier: MIT

use super::super::contract::{
    CommandRequest, CommandResponse, EVENT_SCHEMA_VERSION, EXPORT_SCHEMA_VERSION,
    EventClassification, EventPage, EventType, ExportResponse, MANAGEMENT_SCHEMA_VERSION,
    MAX_EVENTS_PER_PAGE, MAX_RESPONSE_BYTES, PersistedCommand, RunEvent, RunSnapshot,
};
use super::{CommandAcceptance, CommandApplication, StoreCore, StoreError, SubmissionLookup};

#[path = "store_events.rs"]
mod event_ops;

pub(crate) use event_ops::validate_initial_run;
use event_ops::{append_event, classification_for_outcome, management_event, next_sequence};

#[path = "store_persist.rs"]
mod persistence;
#[path = "store_ops_run.rs"]
mod run;

pub(super) use persistence::{io_store_error, persist};
pub(crate) use run::create_run;

const REDACTED_FIELD: &str = "[redacted]";

pub(super) fn lookup_submission(
    core: &StoreCore,
    request_id: &str,
    request_digest: &str,
) -> Result<SubmissionLookup, StoreError> {
    let state = core.read()?;
    match state.submissions.get(request_id) {
        None => Ok(SubmissionLookup::Missing),
        Some(index) if index.request_digest == request_digest => state
            .runs
            .get(&index.workflow_run_id)
            .map(|run| SubmissionLookup::Existing(Box::new(run.snapshot.clone())))
            .ok_or_else(|| {
                StoreError::new(
                    "store_corrupt",
                    "submission index points to a missing workflow run",
                )
            }),
        Some(_) => Ok(SubmissionLookup::Conflict),
    }
}

pub(super) fn get_run(core: &StoreCore, run_id: &str) -> Result<Option<RunSnapshot>, StoreError> {
    Ok(core
        .read()?
        .runs
        .get(run_id)
        .map(|run| run.snapshot.clone()))
}

pub(super) fn events(
    core: &StoreCore,
    run_id: &str,
    after_sequence: u64,
    limit: u64,
) -> Result<EventPage, StoreError> {
    let state = core.read()?;
    let run = state
        .runs
        .get(run_id)
        .ok_or_else(|| StoreError::new("run_not_found", "workflow run was not found"))?;
    let bounded_limit = limit.clamp(1, MAX_EVENTS_PER_PAGE);
    let oldest_sequence = run.oldest_sequence;
    let newest_sequence = run.events.last().map(|event| event.sequence);
    let gap = oldest_sequence
        .filter(|oldest| after_sequence.saturating_add(1) < *oldest)
        .map(|oldest| super::super::contract::EventGap {
            requested_after_sequence: after_sequence,
            oldest_sequence: oldest,
            newest_sequence: newest_sequence.unwrap_or(oldest),
        });
    let events = run
        .events
        .iter()
        .filter(|event| event.sequence > after_sequence)
        .take(bounded_limit as usize)
        .cloned()
        .collect::<Vec<_>>();
    let next_after_sequence = events
        .last()
        .map(|event| event.sequence)
        .unwrap_or(after_sequence);
    Ok(EventPage {
        schema_version: EVENT_SCHEMA_VERSION.to_owned(),
        workflow_run_id: run_id.to_owned(),
        after_sequence,
        oldest_sequence,
        newest_sequence,
        next_after_sequence,
        gap,
        events,
    })
}

pub(super) fn accept_command(
    core: &StoreCore,
    request: &CommandRequest,
    request_digest: &str,
) -> Result<CommandAcceptance, StoreError> {
    core.mutate(|state| {
        let run = state
            .runs
            .get_mut(&request.run_id)
            .ok_or_else(|| StoreError::new("run_not_found", "workflow run was not found"))?;
        if let Some(existing) = run.commands.get(&request.command_id) {
            if existing.request_digest != request_digest {
                return Ok(CommandAcceptance::Conflict);
            }
            return Ok(CommandAcceptance::Existing {
                snapshot: run.snapshot.clone(),
                response: existing.response.clone(),
                application_in_flight: existing.application_in_flight,
            });
        }
        if run
            .commands
            .values()
            .any(|command| command.application_in_flight)
        {
            return Ok(CommandAcceptance::Existing {
                snapshot: run.snapshot.clone(),
                response: None,
                application_in_flight: true,
            });
        }
        if request.expected_revision != run.snapshot.run_revision {
            return Err(StoreError::new(
                "stale_revision",
                "command expected revision does not match the durable run revision",
            ));
        }
        let sequence = next_sequence(run)?;
        let event = management_event(
            &run.snapshot,
            sequence,
            EventType::CommandRequested,
            request.kind.reason_code().to_owned(),
            EventClassification::Accepted,
        );
        append_event(run, event)?;
        run.commands.insert(
            request.command_id.clone(),
            PersistedCommand {
                request_digest: request_digest.to_owned(),
                application_in_flight: true,
                response: None,
            },
        );
        Ok(CommandAcceptance::New {
            snapshot: run.snapshot.clone(),
            requested_sequence: sequence,
        })
    })
}

pub(super) fn apply_command(
    core: &StoreCore,
    request: &CommandRequest,
    request_digest: &str,
    application: CommandApplication,
) -> Result<CommandResponse, StoreError> {
    core.mutate(|state| {
        let run = state
            .runs
            .get_mut(&request.run_id)
            .ok_or_else(|| StoreError::new("run_not_found", "workflow run was not found"))?;
        let command = run
            .commands
            .get(&request.command_id)
            .ok_or_else(|| StoreError::new("command_not_found", "command was not accepted"))?;
        if command.request_digest != request_digest {
            return Err(StoreError::new(
                "command_conflict",
                "command ID was reused with a different payload",
            ));
        }
        if let Some(response) = &command.response {
            return Ok(response.clone());
        }
        if application.snapshot.workflow_run_id != request.run_id {
            return Err(StoreError::new(
                "port_identity_mismatch",
                "execution port returned a different workflow run",
            ));
        }
        let expected_revision = run
            .snapshot
            .run_revision
            .checked_add(1)
            .ok_or_else(|| StoreError::new("revision_overflow", "run revision overflowed"))?;
        if application.snapshot.run_revision != expected_revision {
            return Err(StoreError::new(
                "port_revision_mismatch",
                "execution port returned an unexpected run revision",
            ));
        }
        let sequence = next_sequence(run)?;
        let mut snapshot = application.snapshot;
        snapshot.schema_version = super::super::contract::RUN_SCHEMA_VERSION.to_owned();
        let event = management_event(
            &snapshot,
            sequence,
            EventType::CommandApplied,
            application.reason_code,
            classification_for_outcome(&application.outcome),
        );
        append_event(run, event)?;
        run.snapshot = snapshot.clone();
        let response = CommandResponse {
            schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
            command_id: request.command_id.clone(),
            workflow_run_id: request.run_id.clone(),
            outcome: application.outcome,
            run_revision: snapshot.run_revision,
            sequence: Some(sequence),
        };
        let command = run
            .commands
            .get_mut(&request.command_id)
            .ok_or_else(|| StoreError::new("command_not_found", "command was not accepted"))?;
        command.application_in_flight = false;
        command.response = Some(response.clone());
        Ok(response)
    })
}

pub(super) fn release_command(
    core: &StoreCore,
    request: &CommandRequest,
    request_digest: &str,
) -> Result<(), StoreError> {
    core.mutate(|state| {
        let run = state
            .runs
            .get_mut(&request.run_id)
            .ok_or_else(|| StoreError::new("run_not_found", "workflow run was not found"))?;
        let command = run
            .commands
            .get_mut(&request.command_id)
            .ok_or_else(|| StoreError::new("command_not_found", "command was not accepted"))?;
        if command.request_digest != request_digest {
            return Err(StoreError::new(
                "command_conflict",
                "command ID was reused with a different payload",
            ));
        }
        if command.response.is_none() {
            command.application_in_flight = false;
        }
        Ok(())
    })
}

pub(super) fn export(
    core: &StoreCore,
    run_id: &str,
    redacted: bool,
) -> Result<ExportResponse, StoreError> {
    if !redacted {
        return Err(StoreError::new(
            "redaction_required",
            "management export requires redacted=true",
        ));
    }
    let state = core.read()?;
    let run = state
        .runs
        .get(run_id)
        .ok_or_else(|| StoreError::new("run_not_found", "workflow run was not found"))?;
    let mut redacted_run = run.snapshot.clone();
    redacted_run.cursor.graph_id = REDACTED_FIELD.to_owned();
    redacted_run.cursor.node_id = REDACTED_FIELD.to_owned();
    redacted_run.cursor.node_execution_id = REDACTED_FIELD.to_owned();
    redacted_run.pending_operation = None;
    let redacted_events = run.events.iter().map(redact_event).collect::<Vec<_>>();
    let export = ExportResponse {
        schema_version: EXPORT_SCHEMA_VERSION.to_owned(),
        workflow_run_id: run_id.to_owned(),
        redacted: true,
        run: redacted_run,
        events: redacted_events,
    };
    let bytes = serde_json::to_vec(&export)
        .map_err(|error| StoreError::new("store_encode", error.to_string()))?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(StoreError::new(
            "response_too_large",
            "redacted workflow export exceeds the response limit",
        ));
    }
    Ok(export)
}

fn redact_event(event: &RunEvent) -> RunEvent {
    let mut redacted = event.clone();
    redacted.node_execution_id = REDACTED_FIELD.to_owned();
    redacted.payload.operation_id = None;
    redacted.payload.reason_code = "event".to_owned();
    redacted.integrity_digest = None;
    redacted
}
