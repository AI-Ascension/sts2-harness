// SPDX-License-Identifier: MIT

use super::super::super::contract::{
    EVENT_SCHEMA_VERSION, EventClassification, EventPayload, EventType, MAX_EVENTS_PER_RUN,
    PersistedRun, RunEvent, RunSnapshot,
};
use super::super::StoreError;

pub(super) fn validate_initial_run(
    snapshot: &RunSnapshot,
    events: &[RunEvent],
) -> Result<(), StoreError> {
    if snapshot.schema_version != super::super::super::contract::RUN_SCHEMA_VERSION
        || snapshot.run_revision == 0
        || events.is_empty()
        || events.first().map(|event| event.sequence) != Some(1)
        || events.last().map(|event| event.run_revision) != Some(snapshot.run_revision)
    {
        return Err(StoreError::new(
            "invalid_initial_run",
            "initial workflow snapshot/event sequence is invalid",
        ));
    }
    for (index, event) in events.iter().enumerate() {
        let expected = (index as u64).saturating_add(1);
        if event.sequence != expected || event.workflow_run_id != snapshot.workflow_run_id {
            return Err(StoreError::new(
                "invalid_initial_events",
                "initial workflow events are not contiguous",
            ));
        }
    }
    Ok(())
}

pub(super) fn next_sequence(run: &PersistedRun) -> Result<u64, StoreError> {
    let sequence = run
        .events
        .last()
        .map(|event| event.sequence)
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| StoreError::new("sequence_overflow", "event sequence overflowed"))?;
    if run.events.len() >= MAX_EVENTS_PER_RUN {
        return Err(StoreError::new(
            "event_limit",
            "workflow event retention limit has been reached",
        ));
    }
    Ok(sequence)
}

pub(super) fn append_event(run: &mut PersistedRun, event: RunEvent) -> Result<(), StoreError> {
    if run
        .events
        .last()
        .map(|previous| event.sequence != previous.sequence.saturating_add(1))
        .unwrap_or(event.sequence != 1)
    {
        return Err(StoreError::new(
            "event_sequence_conflict",
            "event sequence is not monotonic",
        ));
    }
    run.events.push(event);
    if run.oldest_sequence.is_none() {
        run.oldest_sequence = run.events.first().map(|item| item.sequence);
    }
    Ok(())
}

pub(super) fn management_event(
    snapshot: &RunSnapshot,
    sequence: u64,
    event_type: EventType,
    reason_code: String,
    classification: EventClassification,
) -> RunEvent {
    RunEvent {
        schema_version: EVENT_SCHEMA_VERSION.to_owned(),
        workflow_run_id: snapshot.workflow_run_id.clone(),
        sequence,
        run_revision: snapshot.run_revision,
        event_type,
        definition_digest: snapshot.definition_digest.clone(),
        node_execution_id: "management".to_owned(),
        payload: EventPayload {
            operation_id: None,
            classification: Some(classification),
            reason_code,
        },
    }
}

pub(super) fn classification_for_outcome(
    outcome: &super::super::super::contract::CommandOutcome,
) -> EventClassification {
    match outcome {
        super::super::super::contract::CommandOutcome::Accepted
        | super::super::super::contract::CommandOutcome::Applied
        | super::super::super::contract::CommandOutcome::Duplicate => EventClassification::Settled,
        super::super::super::contract::CommandOutcome::Pending => EventClassification::Unknown,
    }
}
