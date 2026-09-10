// SPDX-License-Identifier: MIT

use super::super::super::super::contract::{ExportResponse, MAX_RESPONSE_BYTES};
use super::super::super::StoreError;
use super::super::SqliteWorkflowStore;
use super::super::support::{connection, encode, read_events, read_snapshot};

pub(crate) fn export(
    store: &SqliteWorkflowStore,
    run_id: &str,
    redacted: bool,
) -> Result<ExportResponse, StoreError> {
    if !redacted {
        return Err(StoreError::new(
            "redaction_required",
            "management export requires redacted=true",
        ));
    }
    let connection = connection(store)?;
    let run = read_snapshot(&connection, run_id)?
        .ok_or_else(|| StoreError::new("run_not_found", "workflow run was not found"))?;
    let mut redacted_run = run;
    redacted_run.cursor.graph_id = "[redacted]".to_owned();
    redacted_run.cursor.node_id = "[redacted]".to_owned();
    redacted_run.cursor.node_execution_id = "[redacted]".to_owned();
    redacted_run.pending_operation = None;
    let events = read_events(&connection, run_id)?
        .into_iter()
        .map(|mut event| {
            event.node_execution_id = "[redacted]".to_owned();
            event.payload.operation_id = None;
            event.payload.reason_code = "event".to_owned();
            event.integrity_digest = None;
            event
        })
        .collect::<Vec<_>>();
    let export = ExportResponse {
        schema_version: super::super::super::super::contract::EXPORT_SCHEMA_VERSION.to_owned(),
        workflow_run_id: run_id.to_owned(),
        redacted: true,
        run: redacted_run,
        events,
    };
    if encode(&export)?.len() > MAX_RESPONSE_BYTES {
        return Err(StoreError::new(
            "response_too_large",
            "redacted workflow export exceeds the response limit",
        ));
    }
    Ok(export)
}
