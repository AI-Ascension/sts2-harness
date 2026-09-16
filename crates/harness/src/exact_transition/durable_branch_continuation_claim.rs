// SPDX-License-Identifier: MIT

//! Durable owner-claim evidence for one selected branch continuation.

use rusqlite::{OptionalExtension, TransactionBehavior, params};

use super::branch_continuation::BranchContinuationSelector;
use super::{BranchStoreError, SqliteBranchStore};

/// Persisted phase of one branch's current-owner claim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BranchContinuationClaimState {
    /// Operation identity is durable; the owner read has not yet been recorded.
    Prepared,
    /// A non-secret current owner snapshot is durable; gateway claim may be reconciled.
    OwnerSnapshotted,
    /// Gateway acknowledged the exact owner claim.
    Claimed,
    /// An external response was uncertain; read and lookup are required before proceeding.
    Unknown,
    /// The strategy boundary was verified and published for explicit continuation resume.
    BoundaryVerified,
}

impl BranchContinuationClaimState {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::OwnerSnapshotted => "owner_snapshotted",
            Self::Claimed => "claimed",
            Self::Unknown => "unknown",
            Self::BoundaryVerified => "boundary_verified",
        }
    }

    fn parse(value: &str) -> Result<Self, BranchStoreError> {
        match value {
            "prepared" => Ok(Self::Prepared),
            "owner_snapshotted" => Ok(Self::OwnerSnapshotted),
            "claimed" => Ok(Self::Claimed),
            "unknown" => Ok(Self::Unknown),
            "boundary_verified" => Ok(Self::BoundaryVerified),
            _ => Err(BranchStoreError::UnsupportedSchema),
        }
    }
}

/// Persisted owner-claim attempt. `owner_json` contains only the gateway's non-secret fence tuple.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BranchContinuationClaim {
    /// Experiment identity owning the branch.
    pub experiment_id: String,
    /// Selected branch identity.
    pub branch_id: String,
    /// Stable UUID used for every gateway retry and lookup.
    pub operation_id: String,
    /// Durable reconciliation phase.
    pub state: BranchContinuationClaimState,
    /// Exact normalized gateway owner tuple, if read and persisted.
    pub owner_json: Option<String>,
    /// SHA-256 digest of `owner_json`.
    pub owner_digest: Option<String>,
}

impl SqliteBranchStore {
    /// Creates or returns the stable operation identity for a selected branch owner claim.
    ///
    /// A new operation is committed before any gateway request. A retry after process loss
    /// returns the same operation identity; callers must never generate another claim ID for this
    /// branch.
    pub fn prepare_continuation_claim(
        &self,
        experiment_id: &str,
        branch_id: &str,
    ) -> Result<BranchContinuationClaim, BranchStoreError> {
        BranchContinuationSelector::new(experiment_id, branch_id)
            .map_err(|_| BranchStoreError::InvalidInput)?;
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(BranchStoreError::persistence)?;
        if let Some(claim) = read_claim(&transaction, experiment_id, branch_id)? {
            transaction
                .commit()
                .map_err(BranchStoreError::persistence)?;
            return Ok(claim);
        }
        let exists: bool = transaction
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM durable_branches
                    WHERE experiment_id = ?1 AND branch_id = ?2
                )",
                params![experiment_id, branch_id],
                |row| row.get(0),
            )
            .map_err(BranchStoreError::persistence)?;
        if !exists {
            return Err(BranchStoreError::UnknownBranch);
        }
        let operation_id = uuid::Uuid::new_v4().to_string();
        let now = now_millis()?;
        let payload_digest = crate::sha256_hex(
            format!("continuation-owner-claim\0{experiment_id}\0{branch_id}").as_bytes(),
        );
        transaction
            .execute(
                "INSERT INTO branch_operations(
                    operation_id, operation_kind, payload_digest, experiment_id, branch_id, created_at
                 ) VALUES (?1, 'continuation_owner_claim', ?2, ?3, ?4, ?5)",
                params![operation_id, payload_digest, experiment_id, branch_id, now],
            )
            .map_err(BranchStoreError::persistence)?;
        transaction
            .execute(
                "INSERT INTO branch_continuation_claims(
                    experiment_id, branch_id, operation_id, claim_state,
                    owner_json, owner_digest, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, 'prepared', NULL, NULL, ?4, ?4)",
                params![experiment_id, branch_id, operation_id, now],
            )
            .map_err(BranchStoreError::persistence)?;
        let claim = read_claim(&transaction, experiment_id, branch_id)?.ok_or(
            BranchStoreError::Persistence("continuation claim insert disappeared".to_owned()),
        )?;
        transaction
            .commit()
            .map_err(BranchStoreError::persistence)?;
        Ok(claim)
    }

    /// Records the exact non-secret current gateway owner before issuing a claim.
    ///
    /// Re-recording the same tuple is idempotent. A different tuple is rejected so a lost claim
    /// acknowledgement cannot silently retarget an existing operation to a sibling lease.
    pub fn snapshot_continuation_owner(
        &self,
        operation_id: &str,
        owner_json: &str,
    ) -> Result<BranchContinuationClaim, BranchStoreError> {
        if owner_json.is_empty() || owner_json.len() > 8 * 1024 {
            return Err(BranchStoreError::InvalidInput);
        }
        let normalized: serde_json::Value =
            serde_json::from_str(owner_json).map_err(|_| BranchStoreError::InvalidInput)?;
        if !normalized.is_object() {
            return Err(BranchStoreError::InvalidInput);
        }
        let owner_json =
            serde_json::to_string(&normalized).map_err(|_| BranchStoreError::InvalidInput)?;
        let owner_digest = crate::sha256_hex(owner_json.as_bytes());
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(BranchStoreError::persistence)?;
        let current = read_claim_by_operation(&transaction, operation_id)?
            .ok_or(BranchStoreError::UnknownBranch)?;
        if let Some(existing_digest) = current.owner_digest.as_deref() {
            if existing_digest != owner_digest || current.owner_json.as_deref() != Some(&owner_json)
            {
                return Err(BranchStoreError::IdempotencyConflict);
            }
            transaction
                .commit()
                .map_err(BranchStoreError::persistence)?;
            return Ok(current);
        }
        if current.state != BranchContinuationClaimState::Prepared {
            return Err(BranchStoreError::InvalidTransition);
        }
        let now = now_millis()?;
        transaction
            .execute(
                "UPDATE branch_continuation_claims
                 SET claim_state = 'owner_snapshotted', owner_json = ?2,
                     owner_digest = ?3, updated_at = ?4
                 WHERE operation_id = ?1 AND claim_state = 'prepared'",
                params![operation_id, owner_json, owner_digest, now],
            )
            .map_err(BranchStoreError::persistence)?;
        let updated = read_claim_by_operation(&transaction, operation_id)?
            .ok_or(BranchStoreError::UnknownBranch)?;
        transaction
            .commit()
            .map_err(BranchStoreError::persistence)?;
        Ok(updated)
    }

    /// Advances a persisted owner claim through a checked phase transition.
    pub fn transition_continuation_claim(
        &self,
        operation_id: &str,
        expected: BranchContinuationClaimState,
        next: BranchContinuationClaimState,
    ) -> Result<BranchContinuationClaim, BranchStoreError> {
        let allowed = matches!(
            (expected, next),
            (
                BranchContinuationClaimState::OwnerSnapshotted,
                BranchContinuationClaimState::Claimed | BranchContinuationClaimState::Unknown
            ) | (
                BranchContinuationClaimState::Unknown,
                BranchContinuationClaimState::Claimed
                    | BranchContinuationClaimState::BoundaryVerified
            ) | (
                BranchContinuationClaimState::Claimed,
                BranchContinuationClaimState::Unknown
                    | BranchContinuationClaimState::BoundaryVerified
            )
        );
        if !allowed {
            return Err(BranchStoreError::InvalidTransition);
        }
        let mut connection = self.lock()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(BranchStoreError::persistence)?;
        let current = read_claim_by_operation(&transaction, operation_id)?
            .ok_or(BranchStoreError::UnknownBranch)?;
        if current.state == next {
            transaction
                .commit()
                .map_err(BranchStoreError::persistence)?;
            return Ok(current);
        }
        if current.state != expected {
            return Err(BranchStoreError::StaleRevision);
        }
        let now = now_millis()?;
        transaction
            .execute(
                "UPDATE branch_continuation_claims
                 SET claim_state = ?2, updated_at = ?3
                 WHERE operation_id = ?1 AND claim_state = ?4",
                params![operation_id, next.as_str(), now, expected.as_str()],
            )
            .map_err(BranchStoreError::persistence)?;
        let updated = read_claim_by_operation(&transaction, operation_id)?
            .ok_or(BranchStoreError::UnknownBranch)?;
        transaction
            .commit()
            .map_err(BranchStoreError::persistence)?;
        Ok(updated)
    }

    /// Reads the owner-claim journal for one branch, if one has been prepared.
    pub fn continuation_claim(
        &self,
        experiment_id: &str,
        branch_id: &str,
    ) -> Result<Option<BranchContinuationClaim>, BranchStoreError> {
        let connection = self.lock()?;
        read_claim(&connection, experiment_id, branch_id)
    }
}

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
