// SPDX-License-Identifier: MIT

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
