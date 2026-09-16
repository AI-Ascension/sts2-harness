// SPDX-License-Identifier: MIT

#[test]
fn owner_claim_journal_reuses_operation_and_rejects_owner_retargeting()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = TemporaryDirectory::new()?;
    let branch_store = temp.path().join("branches.sqlite3");
    let artifact_path = temp.path().join("artifacts");
    let artifacts = ExactArtifactStore::new(&artifact_path);
    create_store(&branch_store, &artifacts, BranchStrategy::PrefixReplay)?;

    let store = SqliteBranchStore::open(&branch_store)?;
    let first = store.prepare_continuation_claim(EXPERIMENT, "branch:selected")?;
    assert_eq!(first.state, BranchContinuationClaimState::Prepared);
    let retry = store.prepare_continuation_claim(EXPERIMENT, "branch:selected")?;
    assert_eq!(retry.operation_id, first.operation_id);

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
    let serialized = serde_json::to_string(&owner)?;
    let snapshotted = store.snapshot_continuation_owner(&first.operation_id, &serialized)?;
    assert_eq!(
        snapshotted.state,
        BranchContinuationClaimState::OwnerSnapshotted
    );
    assert_eq!(
        store.snapshot_continuation_owner(&first.operation_id, &serialized)?,
        snapshotted
    );
    let different_owner = serde_json::json!({
        "deployment_id":"00000000-0000-4000-8000-000000000001",
        "instance_id":"00000000-0000-4000-8000-000000000002",
        "instance_incarnation":"00000000-0000-4000-8000-000000000003",
        "boot_id":"00000000-0000-4000-8000-000000000004",
        "authority_generation":7,
        "host_fence_id":"00000000-0000-4000-8000-000000000005",
        "host_fence_generation":3,
        "lease_id":"00000000-0000-4000-8000-000000000006",
        "lease_epoch":8,
        "session_id":"sibling-session",
        "lease_expires_at_millis":1_800_000_000_000_u64
    });
    assert_eq!(
        store.snapshot_continuation_owner(
            &first.operation_id,
            &serde_json::to_string(&different_owner)?
        ),
        Err(sts2_harness::BranchStoreError::IdempotencyConflict)
    );
    let claimed = store.transition_continuation_claim(
        &first.operation_id,
        BranchContinuationClaimState::OwnerSnapshotted,
        BranchContinuationClaimState::Claimed,
    )?;
    assert_eq!(
        claimed.owner_digest,
        Some(sts2_harness::sha256_hex(
            snapshotted
                .owner_json
                .as_deref()
                .unwrap_or_default()
                .as_bytes()
        ))
    );
    let persisted = SqliteBranchStore::open(&branch_store)?
        .continuation_claim(EXPERIMENT, "branch:selected")?
        .expect("owner claim persists after reopening");
    assert_eq!(persisted, claimed);
    Ok(())
}

#[test]
fn selected_branch_claim_does_not_mutate_sibling_branch() -> Result<(), Box<dyn std::error::Error>>
{
    let temp = TemporaryDirectory::new()?;
    let branch_store = temp.path().join("branches.sqlite3");
    let artifact_path = temp.path().join("artifacts");
    let artifacts = ExactArtifactStore::new(&artifact_path);
    create_store(&branch_store, &artifacts, BranchStrategy::PrefixReplay)?;

    let store = SqliteBranchStore::open(&branch_store)?;
    let sibling = store.create(
        "operation:create-sibling",
        draft(
            "branch:sibling",
            Some(ROOT),
            BranchStrategy::PrefixReplay,
            Vec::new(),
        ),
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

    let sibling_after = store
        .get(EXPERIMENT, "branch:sibling")?
        .expect("sibling branch remains");
    assert_eq!(sibling_after, sibling);
    assert!(
        store
            .continuation_claim(EXPERIMENT, "branch:sibling")?
            .is_none()
    );
    Ok(())
}
