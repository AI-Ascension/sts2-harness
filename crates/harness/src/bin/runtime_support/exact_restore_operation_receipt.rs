// SPDX-License-Identifier: MIT

use serde_json::Value;

use super::super::super::super::branch_continuation_runtime::SelectedBranchContinuation;
use super::super::super::{VerifiedClosure, branch_payload, canonical_bytes};
use super::super::{failure, uncertain};

pub(crate) fn verify_receipt<'a>(
    receipt: &'a Value,
    selected: &SelectedBranchContinuation,
    closure: &VerifiedClosure,
    expected_owner: &Value,
    operation_id: &str,
) -> Result<&'a Value, super::super::super::ExactRestoreError> {
    let mut unsigned = receipt.clone();
    let supplied = unsigned["receipt_digest"]
        .as_str()
        .ok_or_else(|| uncertain("receipt omitted receipt_digest"))?
        .to_owned();
    unsigned
        .as_object_mut()
        .ok_or_else(|| uncertain("receipt is not an object"))?
        .remove("receipt_digest");
    let actual = format!(
        "sha256:{}",
        sts2_harness::sha256_hex(&canonical_bytes(&unsigned).map_err(uncertain)?)
    );
    if supplied != actual
        || receipt["operation_id"] != operation_id
        || receipt["branch"] != branch_payload(selected)
        || receipt["destination_owner"] != *expected_owner
        || receipt["checkpoint_id"] != closure.checkpoint_id
        || receipt["exact_state_digest"] != closure.exact_state_digest
        || receipt["recaptured_exact_state_digest"] != closure.exact_state_digest
        || receipt["manifest_digest"] != closure.manifest_digest
        || receipt["closure_digest"] != closure.closure_digest
        || receipt["compatibility_digest"] != closure.compatibility_digest
        || receipt["coverage_contract_digest"] != closure.coverage_contract_digest
        || receipt["boundary"] != closure.boundary
        || receipt["artifact_reference_count"] != closure.artifact_reference_count
        || receipt["distinct_blob_count"] != closure.distinct_blob_count
        || receipt["aggregate_closure_bytes"] != closure.aggregate_closure_bytes
    {
        return Err(uncertain(
            "receipt is not bound to the selected branch, owner, closure, and recaptured state",
        ));
    }
    Ok(receipt)
}

pub(super) fn classify_error_response(
    frame: &Value,
    phase: &str,
) -> super::super::super::ExactRestoreError {
    let payload = &frame["payload"];
    let outcome = payload["outcome"].as_str().unwrap_or("unknown");
    let code = payload["error_code"].as_str().unwrap_or("unknown");
    if outcome == "REJECTED" && payload["host_effect"] == "not_started" {
        failure(
            super::super::super::FailureSafety::NotStarted,
            format!("exact-restore {phase} was refused before effects: {code}"),
        )
    } else {
        failure(
            super::super::super::FailureSafety::Uncertain,
            format!("exact-restore {phase} is unresolved: outcome={outcome}, code={code}"),
        )
    }
}

/// A lookup is a reconciliation read after an uncertain commit.  Even a
/// protocol rejection that says `not_started` describes only that lookup
/// request; it cannot prove that the earlier commit had no host effect.
pub(super) fn classify_lookup_error_response(
    frame: &Value,
) -> super::super::super::ExactRestoreError {
    let payload = &frame["payload"];
    let outcome = payload["outcome"].as_str().unwrap_or("unknown");
    let code = payload["error_code"].as_str().unwrap_or("unknown");
    uncertain(format!(
        "same-operation lookup is unresolved: outcome={outcome}, code={code}"
    ))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::classify_lookup_error_response;
    use crate::runtime_support::exact_restore::FailureSafety;

    #[test]
    fn lookup_rejection_after_commit_uncertainty_stays_uncertain() {
        let error = classify_lookup_error_response(&json!({
            "payload": {
                "outcome": "REJECTED",
                "error_code": "owner_mismatch",
                "host_effect": "not_started"
            }
        }));
        assert_eq!(error.safety, FailureSafety::Uncertain);
    }
}
