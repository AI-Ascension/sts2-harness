// SPDX-License-Identifier: MIT

fn read_claim(
    connection: &rusqlite::Connection,
    experiment_id: &str,
    branch_id: &str,
) -> Result<Option<BranchContinuationClaim>, BranchStoreError> {
    connection
        .query_row(
            "SELECT experiment_id, branch_id, operation_id, claim_state, owner_json, owner_digest
             FROM branch_continuation_claims
             WHERE experiment_id = ?1 AND branch_id = ?2",
            params![experiment_id, branch_id],
            claim_from_row,
        )
        .optional()
        .map_err(BranchStoreError::persistence)
}

fn read_claim_by_operation(
    connection: &rusqlite::Connection,
    operation_id: &str,
) -> Result<Option<BranchContinuationClaim>, BranchStoreError> {
    connection
        .query_row(
            "SELECT experiment_id, branch_id, operation_id, claim_state, owner_json, owner_digest
             FROM branch_continuation_claims
             WHERE operation_id = ?1",
            [operation_id],
            claim_from_row,
        )
        .optional()
        .map_err(BranchStoreError::persistence)
}

fn claim_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<BranchContinuationClaim> {
    let state: String = row.get(3)?;
    Ok(BranchContinuationClaim {
        experiment_id: row.get(0)?,
        branch_id: row.get(1)?,
        operation_id: row.get(2)?,
        state: BranchContinuationClaimState::parse(&state).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        owner_json: row.get(4)?,
        owner_digest: row.get(5)?,
    })
}

fn now_millis() -> Result<i64, BranchStoreError> {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| BranchStoreError::InvalidInput)?;
    i64::try_from(duration.as_millis()).map_err(|_| BranchStoreError::InvalidInput)
}
