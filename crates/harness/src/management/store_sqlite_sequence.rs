// SPDX-License-Identifier: MIT

use super::{MAX_EVENTS_PER_RUN, StoreError};

pub(in crate::management::store::sqlite) fn next_sequence(
    transaction: &rusqlite::Transaction<'_>,
    run_id: &str,
) -> Result<u64, StoreError> {
    let sequence = transaction
        .query_row(
            "SELECT COALESCE(MAX(sequence), 0) + 1 FROM management_events
             WHERE workflow_run_id = ?1",
            [run_id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(super::sqlite_error)?;
    let sequence = u64::try_from(sequence)
        .map_err(|_| StoreError::new("sequence_overflow", "event sequence overflowed"))?;
    if sequence == 0 || sequence > MAX_EVENTS_PER_RUN as u64 {
        return Err(StoreError::new(
            "event_limit",
            "workflow event retention limit has been reached",
        ));
    }
    Ok(sequence)
}
