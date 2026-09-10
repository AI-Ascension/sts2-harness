// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use super::super::super::contract::{
    MAX_EVENTS_PER_RUN, PersistedRun, RunEvent, RunSnapshot, SubmissionIndex,
};
use super::super::{StoreCore, StoreError};
use super::validate_initial_run;

pub(crate) fn create_run(
    core: &StoreCore,
    request_id: &str,
    request_digest: &str,
    snapshot: RunSnapshot,
    initial_events: Vec<RunEvent>,
) -> Result<(), StoreError> {
    let initial_events = initial_events
        .into_iter()
        .map(|event| {
            event
                .seal_integrity()
                .map_err(|error| StoreError::new("event_integrity", error))
        })
        .collect::<Result<Vec<_>, _>>()?;
    core.mutate(|state| {
        if state.submissions.contains_key(request_id)
            || state.runs.contains_key(&snapshot.workflow_run_id)
        {
            return Err(StoreError::new(
                "duplicate_run",
                "workflow run or submission already exists",
            ));
        }
        validate_initial_run(&snapshot, &initial_events)?;
        if initial_events.len() > MAX_EVENTS_PER_RUN {
            return Err(StoreError::new(
                "event_limit",
                "initial workflow event set exceeds the supported bound",
            ));
        }
        state.submissions.insert(
            request_id.to_owned(),
            SubmissionIndex {
                request_digest: request_digest.to_owned(),
                workflow_run_id: snapshot.workflow_run_id.clone(),
            },
        );
        state.runs.insert(
            snapshot.workflow_run_id.clone(),
            PersistedRun {
                request_id: request_id.to_owned(),
                request_digest: request_digest.to_owned(),
                snapshot,
                oldest_sequence: initial_events.first().map(|event| event.sequence),
                events: initial_events,
                commands: BTreeMap::new(),
            },
        );
        Ok(())
    })
}
