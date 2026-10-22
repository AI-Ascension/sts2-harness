// SPDX-License-Identifier: MIT

use super::super::contract::{
    MAX_SUBMISSION_CLAIMS, SubmissionClaim, SubmissionClaimAcceptance, validate_digest,
    validate_identifier,
};
use super::{StoreCore, StoreError};

pub(super) fn validate(request_id: &str, claim: &SubmissionClaim) -> Result<(), StoreError> {
    validate_identifier("request_id", request_id)
        .and_then(|()| validate_digest("actor_digest", &claim.actor_digest))
        .and_then(|()| validate_digest("request_digest", &claim.request_digest))
        .map_err(|_| StoreError::new("submission_claim_invalid", "invalid submission claim"))?;
    let bytes = serde_json::to_vec(claim)
        .map_err(|_| StoreError::new("submission_claim_invalid", "invalid submission claim"))?;
    if bytes.len() > 16_384 {
        return Err(StoreError::new(
            "submission_claim_too_large",
            "submission claim exceeds the 16 KiB record bound",
        ));
    }
    Ok(())
}

pub(super) fn accepts(existing: &SubmissionClaim, proposed: &SubmissionClaim) -> bool {
    existing.actor_digest == proposed.actor_digest
        && existing.request_digest == proposed.request_digest
}

pub(super) fn conflict() -> StoreError {
    StoreError::new(
        "submission_claim_conflict",
        "submission claim identity or outcome conflicts",
    )
}

pub(super) fn capacity() -> StoreError {
    StoreError::new(
        "submission_claim_capacity",
        "submission claim capacity is exhausted",
    )
}

pub(super) fn claim(
    core: &StoreCore,
    request_id: &str,
    proposed: SubmissionClaim,
) -> Result<SubmissionClaimAcceptance, StoreError> {
    validate(request_id, &proposed)?;
    if proposed.outcome.is_some() {
        return Err(conflict());
    }
    core.mutate(|state| {
        if let Some(existing) = state.submission_claims.get(request_id) {
            return Ok(if accepts(existing, &proposed) {
                SubmissionClaimAcceptance::Existing(existing.clone())
            } else {
                SubmissionClaimAcceptance::Conflict
            });
        }
        // A legacy row has no authenticated owner. Never adopt it on an ID/digest guess.
        if state.submissions.contains_key(request_id) {
            return Ok(SubmissionClaimAcceptance::Conflict);
        }
        if state.submission_claims.len() >= MAX_SUBMISSION_CLAIMS {
            return Err(capacity());
        }
        state
            .submission_claims
            .insert(request_id.to_owned(), proposed);
        Ok(SubmissionClaimAcceptance::Acquired)
    })
}

pub(super) fn read(
    core: &StoreCore,
    request_id: &str,
) -> Result<Option<SubmissionClaim>, StoreError> {
    Ok(core.read()?.submission_claims.get(request_id).cloned())
}

pub(super) fn finish(
    core: &StoreCore,
    request_id: &str,
    proposed: SubmissionClaim,
) -> Result<(), StoreError> {
    validate(request_id, &proposed)?;
    core.mutate(|state| {
        let existing = state
            .submission_claims
            .get_mut(request_id)
            .ok_or_else(conflict)?;
        if !accepts(existing, &proposed)
            || proposed.outcome.is_none()
            || existing
                .outcome
                .as_ref()
                .is_some_and(|value| Some(value) != proposed.outcome.as_ref())
        {
            return Err(conflict());
        }
        existing.outcome = proposed.outcome;
        Ok(())
    })
}
