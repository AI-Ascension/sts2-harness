// SPDX-License-Identifier: MIT

use crate::runtime_support::exact_restore::publish_fixture;

#[test]
fn exact_restore_running_resume_reopens_full_receipt_without_repeating_restore()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = TemporaryDirectory::new()?;
    let published = publish_fixture(temp.path())?;
    let selector = BranchContinuationSelector::new(
        "experiment:exact-restore-fixture",
        "branch:selected",
    )?;
    let mut selected = SelectedBranchContinuation::load(
        &selector,
        &published.branch_path,
        &published.artifact_path,
    )?;
    let pins = super::super::exact_restore::ProfilePins::from_values(
        published.compatibility.clone(),
        published.coverage.clone(),
    );
    let closure = super::super::exact_restore::VerifiedClosure::prepare(
        &selected,
        &published.artifact_path,
        pins.clone(),
    )?;
    selected.install_exact_restore(closure.clone())?;
    let claim = selected.prepare_owner_claim()?;
    let owner = serde_json::json!({
        "deployment_id":"00000000-0000-4000-8000-000000000001",
        "instance_id":"00000000-0000-4000-8000-000000000002",
        "instance_incarnation":"00000000-0000-4000-8000-000000000003",
        "boot_id":"00000000-0000-4000-8000-000000000004",
        "authority_generation":9,
        "host_fence_id":"00000000-0000-4000-8000-000000000005",
        "host_fence_generation":4,
        "lease_id":"00000000-0000-4000-8000-000000000006",
        "lease_epoch":12,
        "session_id":"selected-session",
        "lease_expires_at_millis":1_900_000_000_000_u64
    });
    let store = SqliteBranchStore::open(&published.branch_path)?;
    store.snapshot_continuation_owner(&claim.operation_id, &serde_json::to_string(&owner)?)?;
    store.transition_continuation_claim(
        &claim.operation_id,
        BranchContinuationClaimState::OwnerSnapshotted,
        BranchContinuationClaimState::Claimed,
    )?;
    // Refresh the in-memory claim after the owner-facing Gateway acknowledgement.
    let claim = selected.prepare_owner_claim()?;
    selected.claim_exact_restore()?;

    let begin_payload =
        closure.begin_payload_for_test(&selected, &claim.operation_id, owner.clone())?;
    let mut receipt = serde_json::json!({
        "operation_id": claim.operation_id,
        "branch": begin_payload["branch"],
        "destination_owner": owner,
        "checkpoint_id": closure.checkpoint_id,
        "exact_state_digest": closure.exact_state_digest,
        "manifest_digest": closure.manifest_digest,
        "closure_digest": closure.closure_digest,
        "compatibility_digest": closure.compatibility_digest,
        "coverage_contract_digest": closure.coverage_contract_digest,
        "aggregate_closure_bytes": closure.aggregate_closure_bytes,
        "artifact_reference_count": closure.artifact_reference_count,
        "distinct_blob_count": closure.distinct_blob_count,
        "boundary": closure.boundary,
        "recaptured_exact_state_digest": closure.exact_state_digest,
    });
    let unsigned = super::super::exact_restore::canonical_bytes(&receipt)?;
    receipt["receipt_digest"] =
        serde_json::json!(format!("sha256:{}", sts2_harness::sha256_hex(&unsigned)));
    let receipt_bytes = super::super::exact_restore::canonical_bytes(&receipt)?;
    selected.publish_exact_restore_receipt(&receipt_bytes)?;
    let receipt_revision = receipt["branch"]["metadata_revision"]
        .as_u64()
        .ok_or("receipt omitted original branch revision")?;
    let running_revision = selected.branch().metadata_revision;
    assert!(running_revision > receipt_revision);
    drop(selected);

    // Reopen the durable store and advance mutable branch metadata after the receipt was made.
    // The receipt must retain the original claim revision while the current CAS token changes.
    let store = SqliteBranchStore::open(&published.branch_path)?;
    let held = store.transition(
        "operation:resume-reopen-hold",
        "experiment:exact-restore-fixture",
        "branch:selected",
        running_revision,
        DurableBranchStatus::Held,
    )?;
    let current = store.transition(
        "operation:resume-reopen-running",
        "experiment:exact-restore-fixture",
        "branch:selected",
        held.metadata_revision,
        DurableBranchStatus::Running,
    )?;
    assert!(current.metadata_revision > running_revision);
    let events_before_resume = store.events("experiment:exact-restore-fixture", 0, 256)?.events;
    let claim_before_resume = store
        .continuation_claim("experiment:exact-restore-fixture", "branch:selected")?
        .ok_or("resume claim disappeared before reopen")?;
    assert_eq!(
        claim_before_resume.operation_id,
        claim.operation_id,
        "reopen must reuse the original owner-claim operation"
    );
    assert_eq!(
        claim_before_resume.state,
        BranchContinuationClaimState::BoundaryVerified
    );
    drop(store);

    let mut resumed = SelectedBranchContinuation::load_for_resume(
        &selector,
        &published.branch_path,
        &published.artifact_path,
    )?;
    let resumed_closure = super::super::exact_restore::VerifiedClosure::prepare(
        &resumed,
        &published.artifact_path,
        pins,
    )?;
    resumed.verify_persisted_exact_receipt(&resumed_closure)?;
    assert_eq!(resumed.branch().status, DurableBranchStatus::Running);
    assert_eq!(resumed.branch().metadata_revision, current.metadata_revision);
    assert_eq!(
        resumed
            .owner_claim
            .as_ref()
            .map(|claim| claim.operation_id.as_str()),
        Some(claim.operation_id.as_str())
    );

    // A changed original revision is refused even if the attacker recomputes the outer digest.
    let mut tampered_revision: serde_json::Value = serde_json::from_slice(&receipt_bytes)?;
    tampered_revision["branch"]["metadata_revision"] =
        serde_json::json!(receipt_revision + 1);
    let tampered_revision_bytes = {
        let mut unsigned = tampered_revision.clone();
        unsigned
            .as_object_mut()
            .ok_or("receipt must remain an object")?
            .remove("receipt_digest");
        let digest = format!(
            "sha256:{}",
            sts2_harness::sha256_hex(&super::super::exact_restore::canonical_bytes(&unsigned)?)
        );
        tampered_revision["receipt_digest"] = serde_json::json!(digest);
        super::super::exact_restore::canonical_bytes(&tampered_revision)?
    };
    let revision_error = resumed
        .verify_exact_restore_receipt_bytes(&tampered_revision_bytes, &resumed_closure)
        .expect_err("changed original claim revision must be refused");
    assert!(revision_error.contains("does not match its claim history"));

    // A foreign owner fence is refused by the full receipt verifier, even with a valid digest.
    let mut tampered_owner: serde_json::Value = serde_json::from_slice(&receipt_bytes)?;
    tampered_owner["destination_owner"]["lease_id"] =
        serde_json::json!("00000000-0000-4000-8000-000000000099");
    let mut unsigned = tampered_owner.clone();
    unsigned
        .as_object_mut()
        .ok_or("receipt must remain an object")?
        .remove("receipt_digest");
    tampered_owner["receipt_digest"] = serde_json::json!(format!(
        "sha256:{}",
        sts2_harness::sha256_hex(&super::super::exact_restore::canonical_bytes(&unsigned)?)
    ));
    let owner_error = resumed
        .verify_exact_restore_receipt_bytes(
            &super::super::exact_restore::canonical_bytes(&tampered_owner)?,
            &resumed_closure,
        )
        .expect_err("foreign destination owner must be refused");
    assert!(owner_error.contains("not bound to the selected branch, owner, closure"));

    // Resume validation is read-only: no second restore claim/effect is emitted.
    let events_after_verify = SqliteBranchStore::open(&published.branch_path)?
        .events("experiment:exact-restore-fixture", 0, 256)?
        .events;
    assert_eq!(events_after_verify.len(), events_before_resume.len());
    assert_eq!(
        events_after_verify
            .iter()
            .filter(|event| event.operation_id == format!("{}:claim-restore", resumed.operation_id))
            .count(),
        1,
        "resume must retain exactly one durable restore claim"
    );
    assert_eq!(
        SqliteBranchStore::open(&published.branch_path)?
            .continuation_claim("experiment:exact-restore-fixture", "branch:selected")?
            .map(|claim| claim.state),
        Some(BranchContinuationClaimState::BoundaryVerified)
    );
    resumed.claim_resume()?;
    assert_eq!(
        SqliteBranchStore::open(&published.branch_path)?
            .continuation_claim("experiment:exact-restore-fixture", "branch:selected")?
            .map(|claim| claim.state),
        Some(BranchContinuationClaimState::Resuming)
    );
    Ok(())
}

