// SPDX-License-Identifier: MIT

//! Server-owned journal for authoring-inference operations.
//!
//! One authoring operation is identified by `(draft_id, client_mutation_id)`.
//! The journal records the reserved cost *before* any provider call, so a
//! budget-exhausted, cancelled, restarted or lost-response operation keeps
//! exactly one identity and an honest cost/outcome state instead of being
//! silently repeated. A repeated key with byte-identical input replays the
//! recorded outcome; a repeated key with different input is a conflict; a
//! repeated key while the first attempt is still pending is in-progress.
//!
//! The journal is deliberately append-once-per-identity: a terminal state is
//! never rewritten and a terminal operation is never reset to pending, so an
//! operator cannot mistake a retry for a fresh reservation.

use std::collections::BTreeMap;
use std::sync::Mutex;

use super::super::contract_authoring_inference::{
    AUTHORING_INFERENCE_OPERATION_SCHEMA_VERSION, AuthoringInferenceCost,
    AuthoringInferenceOperationRecord, AuthoringInferenceOperationState,
    AuthoringInferenceProposal,
};
use super::super::store::StoreError;

const MAX_DETAIL_BYTES: usize = 256;

/// Stable identity of one authoring operation.
#[must_use]
pub fn authoring_inference_operation_id(draft_id: &str, client_mutation_id: &str) -> String {
    format!("authoring-inference:{draft_id}:{client_mutation_id}")
}

/// The result of beginning one operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthoringInferenceBegin {
    /// The identity was new; the operation may contact the provider.
    Started(Box<AuthoringInferenceOperationRecord>),
    /// The identical operation already reached a terminal state.
    Replayed(Box<AuthoringInferenceOperationRecord>),
    /// The identical operation is still reserved and unresolved.
    InProgress(Box<AuthoringInferenceOperationRecord>),
    /// The identity is held by a different request.
    Conflict(Box<AuthoringInferenceOperationRecord>),
}

/// Server-owned authoring-operation journal.
pub trait AuthoringInferenceJournal: Send + Sync {
    /// Records the reservation before any provider call.
    fn begin(
        &self,
        operation_id: &str,
        draft_id: &str,
        client_mutation_id: &str,
        request_digest: &str,
        reserved: AuthoringInferenceCost,
    ) -> Result<AuthoringInferenceBegin, StoreError>;

    /// Records the terminal state of one operation.
    fn complete(
        &self,
        operation_id: &str,
        state: AuthoringInferenceOperationState,
        cost: AuthoringInferenceCost,
        proposal: Option<Box<AuthoringInferenceProposal>>,
        detail: &str,
    ) -> Result<AuthoringInferenceOperationRecord, StoreError>;

    /// Reads one recorded operation, if it exists.
    fn get(
        &self,
        operation_id: &str,
    ) -> Result<Option<AuthoringInferenceOperationRecord>, StoreError>;
}

#[derive(Default)]
pub struct MemoryAuthoringInferenceJournal {
    records: Mutex<BTreeMap<String, AuthoringInferenceOperationRecord>>,
}

impl MemoryAuthoringInferenceJournal {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(
        &self,
    ) -> Result<
        std::sync::MutexGuard<'_, BTreeMap<String, AuthoringInferenceOperationRecord>>,
        StoreError,
    > {
        self.records.lock().map_err(|_| {
            StoreError::new(
                "authoring_inference_journal_poisoned",
                "authoring-inference journal lock is poisoned",
            )
        })
    }
}

impl AuthoringInferenceJournal for MemoryAuthoringInferenceJournal {
    fn begin(
        &self,
        operation_id: &str,
        draft_id: &str,
        client_mutation_id: &str,
        request_digest: &str,
        reserved: AuthoringInferenceCost,
    ) -> Result<AuthoringInferenceBegin, StoreError> {
        validate_identity(draft_id, client_mutation_id, request_digest)?;
        let mut records = self.lock()?;
        if let Some(existing) = records.get(operation_id).cloned() {
            return Ok(if existing.request_digest != request_digest {
                AuthoringInferenceBegin::Conflict(Box::new(existing))
            } else if existing.state.is_terminal() {
                AuthoringInferenceBegin::Replayed(Box::new(existing))
            } else {
                AuthoringInferenceBegin::InProgress(Box::new(existing))
            });
        }
        let record = AuthoringInferenceOperationRecord {
            schema_version: AUTHORING_INFERENCE_OPERATION_SCHEMA_VERSION.to_owned(),
            operation_id: operation_id.to_owned(),
            draft_id: draft_id.to_owned(),
            client_mutation_id: client_mutation_id.to_owned(),
            request_digest: request_digest.to_owned(),
            state: AuthoringInferenceOperationState::Pending,
            cost: reserved,
            proposal_id: None,
            proposal: None,
            detail: "reserved before provider exchange".to_owned(),
        };
        records.insert(operation_id.to_owned(), record.clone());
        Ok(AuthoringInferenceBegin::Started(Box::new(record)))
    }

    fn complete(
        &self,
        operation_id: &str,
        state: AuthoringInferenceOperationState,
        cost: AuthoringInferenceCost,
        proposal: Option<Box<AuthoringInferenceProposal>>,
        detail: &str,
    ) -> Result<AuthoringInferenceOperationRecord, StoreError> {
        if state == AuthoringInferenceOperationState::Pending {
            return Err(StoreError::new(
                "authoring_inference_journal_invalid_transition",
                "a completed operation cannot be reset to pending",
            ));
        }
        let mut records = self.lock()?;
        let Some(current) = records.get(operation_id).cloned() else {
            return Err(StoreError::new(
                "authoring_inference_operation_not_found",
                "the authoring-inference operation was not reserved",
            ));
        };
        if current.state.is_terminal() && current.state != state {
            return Err(StoreError::new(
                "authoring_inference_journal_invalid_transition",
                "a terminal authoring-inference outcome cannot be rewritten",
            ));
        }
        let updated = AuthoringInferenceOperationRecord {
            state,
            cost,
            proposal_id: proposal.as_ref().map(|value| value.proposal_id.clone()),
            proposal,
            detail: bounded_detail(detail),
            ..current
        };
        records.insert(operation_id.to_owned(), updated.clone());
        Ok(updated)
    }

    fn get(
        &self,
        operation_id: &str,
    ) -> Result<Option<AuthoringInferenceOperationRecord>, StoreError> {
        Ok(self.lock()?.get(operation_id).cloned())
    }
}

/// Refuses every operation when no journal is attached, so a proposal is never
/// accepted into a process-local state an operator cannot see.
pub struct UnavailableAuthoringInferenceJournal;

impl AuthoringInferenceJournal for UnavailableAuthoringInferenceJournal {
    fn begin(
        &self,
        _operation_id: &str,
        _draft_id: &str,
        _client_mutation_id: &str,
        _request_digest: &str,
        _reserved: AuthoringInferenceCost,
    ) -> Result<AuthoringInferenceBegin, StoreError> {
        Err(unavailable())
    }

    fn complete(
        &self,
        _operation_id: &str,
        _state: AuthoringInferenceOperationState,
        _cost: AuthoringInferenceCost,
        _proposal: Option<Box<AuthoringInferenceProposal>>,
        _detail: &str,
    ) -> Result<AuthoringInferenceOperationRecord, StoreError> {
        Err(unavailable())
    }

    fn get(
        &self,
        _operation_id: &str,
    ) -> Result<Option<AuthoringInferenceOperationRecord>, StoreError> {
        Err(unavailable())
    }
}

fn unavailable() -> StoreError {
    StoreError::new(
        "authoring_inference_journal_unavailable",
        "the authoring-inference journal is not injected",
    )
}

fn validate_identity(
    draft_id: &str,
    client_mutation_id: &str,
    request_digest: &str,
) -> Result<(), StoreError> {
    for (field, value) in [
        ("draft_id", draft_id),
        ("client_mutation_id", client_mutation_id),
    ] {
        if value.is_empty() || value.len() > super::super::contract::MAX_IDENTIFIER_BYTES {
            return Err(StoreError::new(
                "authoring_inference_journal_identity",
                format!("{field} is outside its bounds"),
            ));
        }
    }
    if request_digest.len() != 64
        || !request_digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(StoreError::new(
            "authoring_inference_journal_identity",
            "request_digest must be a lowercase SHA-256 digest",
        ));
    }
    Ok(())
}

fn bounded_detail(detail: &str) -> String {
    if detail.len() <= MAX_DETAIL_BYTES {
        return detail.to_owned();
    }
    let mut end = MAX_DETAIL_BYTES;
    while !detail.is_char_boundary(end) {
        end -= 1;
    }
    detail[..end].to_owned()
}
