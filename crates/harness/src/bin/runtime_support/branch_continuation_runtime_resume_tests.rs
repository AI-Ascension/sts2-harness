// SPDX-License-Identifier: MIT

use crate::runtime_support::exact_restore::publish_fixture;

#[cfg(unix)]
#[test]
fn resume_lock_waits_out_a_transient_holder() -> Result<(), Box<dyn std::error::Error>> {
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    let temp = TemporaryDirectory::new()?;
    let branch_store = temp.path().join("branches.sqlite3");
    let artifact_path = temp.path().join("artifacts");
    let artifacts = ExactArtifactStore::new(&artifact_path);
    create_store(&branch_store, &artifacts, BranchStrategy::PrefixReplay)?;

    let (held, held_rx) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    let holder_path = branch_store.clone();
    let holder = thread::spawn(move || {
        let lock = super::resume_lock::acquire(&holder_path, EXPERIMENT, "branch:selected")
            .expect("holder acquires continuation lock");
        held.send(()).expect("holder signal");
        release_rx.recv().expect("release signal");
        drop(lock);
    });
    held_rx.recv().expect("holder reached the lock");

    let releaser = thread::spawn(move || {
        thread::sleep(Duration::from_millis(40));
        release.send(()).expect("release");
    });
    let reacquired =
        super::resume_lock::acquire(&branch_store, EXPERIMENT, "branch:selected")
            .expect("transient descriptor must not be reported as a live owner");
    drop(reacquired);
    releaser.join().expect("releaser thread");
    holder.join().expect("holder thread");
    Ok(())
}

#[test]
fn running_branch_resume_requires_boundary_claim_and_keeps_prefix_unreplayed()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = TemporaryDirectory::new()?;
    let branch_store = temp.path().join("branches.sqlite3");
    let artifact_path = temp.path().join("artifacts");
    let artifacts = ExactArtifactStore::new(&artifact_path);
    create_store(&branch_store, &artifacts, BranchStrategy::PrefixReplay)?;
    let store = SqliteBranchStore::open(&branch_store)?;
    let ready = store
        .get(EXPERIMENT, "branch:selected")?
        .expect("selected branch");
    let running = store.transition(
        "operation:publish-running-for-resume-test",
        EXPERIMENT,
        "branch:selected",
        ready.metadata_revision,
        DurableBranchStatus::Running,
    )?;
    let claim = store.prepare_continuation_claim(EXPERIMENT, "branch:selected")?;
    let owner = serde_json::json!({
        "deployment_id":"00000000-0000-4000-8000-000000000001",
        "instance_id":"00000000-0000-4000-8000-000000000002",
        "instance_incarnation":"00000000-0000-4000-8000-000000000003",
        "boot_id":"00000000-0000-4000-8000-000000000004",
        "authority_generation":7,
        "host_fence_id":"00000000-0000-4000-8000-000000000005",
        "host_fence_generation":3,
        "lease_id":"00000000-0000-4000-8000-000000000006",
        "lease_epoch":8,
        "session_id":"selected-session",
        "lease_expires_at_millis":1_800_000_000_000_u64
    });
    store.snapshot_continuation_owner(&claim.operation_id, &serde_json::to_string(&owner)?)?;
    store.transition_continuation_claim(
        &claim.operation_id,
        BranchContinuationClaimState::OwnerSnapshotted,
        BranchContinuationClaimState::Claimed,
    )?;
    store.transition_continuation_claim(
        &claim.operation_id,
        BranchContinuationClaimState::Claimed,
        BranchContinuationClaimState::BoundaryVerified,
    )?;

    let selector =
        BranchContinuationSelector::new(EXPERIMENT, "branch:selected").expect("selector");
    let resumed =
        SelectedBranchContinuation::load_for_resume(&selector, &branch_store, &artifact_path)?;
    assert!(resumed.is_resuming());
    assert_eq!(resumed.branch().status, DurableBranchStatus::Running);
    assert_eq!(resumed.replay_prefix(), Some(b"{}".as_slice()));
    assert_eq!(
        store
            .continuation_claim(EXPERIMENT, "branch:selected")?
            .map(|claim| claim.state),
        Some(BranchContinuationClaimState::BoundaryVerified)
    );
    let locked = SelectedBranchContinuation::load_for_resume(
        &selector,
        &branch_store,
        &artifact_path,
    )
    .err()
    .expect("second active resume must be excluded");
    assert!(locked.contains("active continuation process"));
    #[cfg(unix)]
    {
        let store_alias = temp.path().join("branch-store-alias.sqlite3");
        std::os::unix::fs::symlink(&branch_store, &store_alias)?;
        let aliased = SelectedBranchContinuation::load_for_resume(
            &selector,
            &store_alias,
            &artifact_path,
        )
        .err()
        .expect("store aliases must share the active resume lock");
        assert!(aliased.contains("active continuation process"));
    }
    assert_eq!(
        store
            .get(EXPERIMENT, "branch:selected")?
            .expect("selected branch remains")
            .metadata_revision,
        running.metadata_revision
    );
    drop(resumed);
    let mut retry = SelectedBranchContinuation::load_for_resume(
        &selector,
        &branch_store,
        &artifact_path,
    )?;
    retry.claim_resume()?;
    assert_eq!(
        store
            .continuation_claim(EXPERIMENT, "branch:selected")?
            .map(|claim| claim.state),
        Some(BranchContinuationClaimState::Resuming)
    );
    Ok(())
}

#[test]
fn exact_restore_resume_reuses_verified_receipt_without_reclaiming_restore()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = TemporaryDirectory::new()?;
    let branch_store = temp.path().join("branches.sqlite3");
    let artifact_path = temp.path().join("artifacts");
    let artifacts = ExactArtifactStore::new(&artifact_path);
    create_store(&branch_store, &artifacts, BranchStrategy::ExactRestore)?;
    let store = SqliteBranchStore::open(&branch_store)?;
    let ready = store
        .get(EXPERIMENT, "branch:selected")?
        .expect("selected branch");
    for index in 0..62 {
        let branch_id = format!("branch:history-noise-{index}");
        let blob = artifacts.stage_blob(format!("noise-{index}").as_bytes())?;
        let noise = store.create(
            &format!("operation:create-{branch_id}"),
            draft(
                &branch_id,
                Some(ROOT),
                BranchStrategy::PrefixReplay,
                vec![sts2_harness::BranchArtifactReference {
                    artifact_id: blob.as_str().to_owned(),
                    role: BranchArtifactRole::ReplayPrefix,
                }],
            ),
        )?;
        let noise = store.transition(
            &format!("operation:replay-{branch_id}"),
            EXPERIMENT,
            &branch_id,
            noise.metadata_revision,
            DurableBranchStatus::Replaying,
        )?;
        let noise = store.set_assurance(
            &format!("operation:assure-{branch_id}"),
            EXPERIMENT,
            &branch_id,
            noise.metadata_revision,
            BranchAssurance::PrefixReplayBoundary,
        )?;
        store.transition(
            &format!("operation:ready-{branch_id}"),
            EXPERIMENT,
            &branch_id,
            noise.metadata_revision,
            DurableBranchStatus::Ready,
        )?;
    }
    let selector =
        BranchContinuationSelector::new(EXPERIMENT, "branch:selected").expect("selector");
    let operation_id = super::operation_id(&selector, ready.metadata_revision);
    let restoring = store.transition(
        &format!("{operation_id}:claim-restore"),
        EXPERIMENT,
        "branch:selected",
        ready.metadata_revision,
        DurableBranchStatus::Restoring,
    )?;
    let receipt_bytes = serde_json::json!({
        "operation_id": "receipt-operation",
        "state": "RESTORE_VERIFIED",
        "branch": {
            "metadata_revision": restoring.metadata_revision,
        },
    });
    let receipt_bytes = serde_json::to_vec(&receipt_bytes)?;
    let receipt_blob = artifacts.stage_blob(&receipt_bytes)?;
    store.attach_artifact(
        "operation:attach-exact-receipt-for-resume-test",
        EXPERIMENT,
        "branch:selected",
        restoring.metadata_revision,
        sts2_harness::BranchArtifactReference {
            artifact_id: receipt_blob.as_str().to_owned(),
            role: BranchArtifactRole::ContextSnapshot,
        },
    )?;
    let assured = store.set_assurance(
        &format!("{operation_id}:restore-assurance"),
        EXPERIMENT,
        "branch:selected",
        restoring.metadata_revision + 1,
        BranchAssurance::ExactRestoreReceipt,
    )?;
    let ready = store.transition(
        &format!("{operation_id}:restore-ready"),
        EXPERIMENT,
        "branch:selected",
        assured.metadata_revision,
        DurableBranchStatus::Ready,
    )?;
    let running = store.transition(
        &format!("{operation_id}:restore-running"),
        EXPERIMENT,
        "branch:selected",
        ready.metadata_revision,
        DurableBranchStatus::Running,
    )?;
    let claim = store.prepare_continuation_claim(EXPERIMENT, "branch:selected")?;
    let owner = serde_json::json!({
        "deployment_id":"00000000-0000-4000-8000-000000000001",
        "instance_id":"00000000-0000-4000-8000-000000000002",
        "instance_incarnation":"00000000-0000-4000-8000-000000000003",
        "boot_id":"00000000-0000-4000-8000-000000000004",
        "authority_generation":7,
        "host_fence_id":"00000000-0000-4000-8000-000000000005",
        "host_fence_generation":3,
        "lease_id":"00000000-0000-4000-8000-000000000006",
        "lease_epoch":8,
        "session_id":"selected-session",
        "lease_expires_at_millis":1_800_000_000_000_u64
    });
    store.snapshot_continuation_owner(&claim.operation_id, &serde_json::to_string(&owner)?)?;
    store.transition_continuation_claim(
        &claim.operation_id,
        BranchContinuationClaimState::OwnerSnapshotted,
        BranchContinuationClaimState::Claimed,
    )?;
    store.transition_continuation_claim(
        &claim.operation_id,
        BranchContinuationClaimState::Claimed,
        BranchContinuationClaimState::BoundaryVerified,
    )?;

    let mut resumed =
        SelectedBranchContinuation::load_for_resume(&selector, &branch_store, &artifact_path)?;
    assert!(resumed.is_resuming());
    assert!(resumed.is_exact_restore());
    assert_eq!(resumed.branch().status, DurableBranchStatus::Running);
    assert_eq!(
        resumed.branch().assurance,
        BranchAssurance::ExactRestoreReceipt
    );
    assert_eq!(
        store
            .continuation_claim(EXPERIMENT, "branch:selected")?
            .map(|claim| claim.state),
        Some(BranchContinuationClaimState::BoundaryVerified)
    );
    resumed.verify_persisted_exact_receipt_revision(&receipt_bytes)?;
    let mut tampered_receipt: serde_json::Value = serde_json::from_slice(&receipt_bytes)?;
    tampered_receipt["branch"]["metadata_revision"] =
        serde_json::json!(restoring.metadata_revision + 1);
    let tampered_receipt = serde_json::to_vec(&tampered_receipt)?;
    let error = resumed
        .verify_persisted_exact_receipt_revision(&tampered_receipt)
        .expect_err("a receipt with a changed original revision must be refused");
    assert!(error.contains("does not match its claim history"));
    resumed.claim_resume()?;
    assert_eq!(
        store
            .continuation_claim(EXPERIMENT, "branch:selected")?
            .map(|claim| claim.state),
        Some(BranchContinuationClaimState::Resuming)
    );
    assert_eq!(
        store
            .get(EXPERIMENT, "branch:selected")?
            .expect("selected branch remains")
            .metadata_revision,
        running.metadata_revision
    );
    Ok(())
}

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

    let mut receipt =
        closure.begin_payload_for_test(&selected, &claim.operation_id, owner.clone())?;
    receipt["state"] = serde_json::json!("RESTORE_VERIFIED");
    receipt["destination_owner"] = owner.clone();
    receipt["recaptured_exact_state_digest"] =
        serde_json::json!(closure.exact_state_digest.clone());
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
    tampered_owner["destination_owner"]["lease_id"] = serde_json::json!("foreign-lease");
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

#[test]
fn running_branch_resume_refuses_missing_boundary_claim() -> Result<(), Box<dyn std::error::Error>>
{
    let temp = TemporaryDirectory::new()?;
    let branch_store = temp.path().join("branches.sqlite3");
    let artifact_path = temp.path().join("artifacts");
    let artifacts = ExactArtifactStore::new(&artifact_path);
    create_store(&branch_store, &artifacts, BranchStrategy::PrefixReplay)?;
    let store = SqliteBranchStore::open(&branch_store)?;
    let ready = store
        .get(EXPERIMENT, "branch:selected")?
        .expect("selected branch");
    store.transition(
        "operation:publish-running-without-claim",
        EXPERIMENT,
        "branch:selected",
        ready.metadata_revision,
        DurableBranchStatus::Running,
    )?;
    let selector =
        BranchContinuationSelector::new(EXPERIMENT, "branch:selected").expect("selector");
    let error =
        SelectedBranchContinuation::load_for_resume(&selector, &branch_store, &artifact_path)
            .err()
            .expect("running branch without claim cannot resume");
    assert!(error.contains("no durable current-owner claim"));
    Ok(())
}
