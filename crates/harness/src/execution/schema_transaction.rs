// SPDX-License-Identifier: MIT

use rusqlite::{Connection, Transaction};

use super::{ExecutionStoreError, map_sqlite};

pub(crate) fn transaction<'a>(
    connection: &'a mut Connection,
) -> Result<Transaction<'a>, ExecutionStoreError> {
    connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(map_sqlite)
}
