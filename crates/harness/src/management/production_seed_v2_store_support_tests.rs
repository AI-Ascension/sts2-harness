// SPDX-License-Identifier: MIT

use crate::management::{SqliteWorkflowStore, WorkflowStore};

pub(super) fn assert_store_corrupt(
    store: &SqliteWorkflowStore,
    request_id: &str,
    actor_digest: &str,
    request_digest: &str,
) {
    assert!(
        matches!(
            store.lookup_seed_operation(request_id, actor_digest, request_digest),
            Err(error) if error.code == "store_corrupt"
        ),
        "inconsistent persisted candidate must fail closed"
    );
}

pub(super) fn table_count(store: &SqliteWorkflowStore, table: &str) -> i64 {
    assert!(matches!(
        table,
        "management_seed_operations"
            | "management_submissions"
            | "management_runs"
            | "management_seed_bindings"
    ));
    store
        .connection
        .lock()
        .expect("SQLite lock")
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("table count")
}
