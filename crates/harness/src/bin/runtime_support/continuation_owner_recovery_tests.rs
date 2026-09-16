// SPDX-License-Identifier: MIT

#[test]
fn lost_claim_ack_reconciles_the_same_operation_without_claiming_a_second_owner()
-> Result<(), String> {
    let (_scratch, config, authority, context) = setup()?;
    let mut gateway = RecordingOwnerGateway {
        owner: owner_for(&config, &authority),
        state: "available",
        claim_seen: false,
        lose_first_claim_ack: true,
        fail_first_claim_before_commit: false,
        calls: Vec::new(),
    };

    claim_current_owner_with_gateway(&config, Some(&authority), &context, &mut gateway)?;

    assert_eq!(
        gateway
            .calls
            .iter()
            .map(|(path, _, _)| path.as_str())
            .collect::<Vec<_>>(),
        [READ_PATH, LOOKUP_PATH, CLAIM_PATH, LOOKUP_PATH]
    );
    assert_eq!(
        gateway.calls[2].2["payload"]["operation_id"],
        context.claim.operation_id
    );
    assert_eq!(
        gateway.calls[3].2["payload"]["operation_id"],
        context.claim.operation_id
    );
    let store =
        SqliteBranchStore::open(&context.branch_store_path).map_err(|error| error.to_string())?;
    let persisted = store
        .continuation_claim(&context.claim.experiment_id, &context.claim.branch_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| String::from("owner claim journal is missing"))?;
    assert_eq!(
        persisted.state,
        sts2_harness::BranchContinuationClaimState::Claimed
    );
    Ok(())
}

#[test]
fn unavailable_or_changed_owner_is_refused_before_claim() -> Result<(), String> {
    let (_scratch, config, authority, context) = setup()?;
    let mut gateway = RecordingOwnerGateway {
        owner: owner_for(&config, &authority),
        state: "unknown",
        claim_seen: false,
        lose_first_claim_ack: false,
        fail_first_claim_before_commit: false,
        calls: Vec::new(),
    };
    assert!(
        claim_current_owner_with_gateway(&config, Some(&authority), &context, &mut gateway)
            .is_err()
    );
    assert_eq!(gateway.calls.len(), 1);
    assert_eq!(gateway.calls[0].0, READ_PATH);
    assert!(!gateway.claim_seen);
    let store =
        SqliteBranchStore::open(&context.branch_store_path).map_err(|error| error.to_string())?;
    let persisted = store
        .continuation_claim(&context.claim.experiment_id, &context.claim.branch_id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| String::from("owner claim intent is missing"))?;
    assert_eq!(
        persisted.state,
        sts2_harness::BranchContinuationClaimState::Prepared
    );
    Ok(())
}

#[test]
fn no_commit_ack_retries_only_the_persisted_id_after_live_fence_recheck() -> Result<(), String> {
    let (_scratch, config, authority, context) = setup()?;
    let mut gateway = RecordingOwnerGateway {
        owner: owner_for(&config, &authority),
        state: "available",
        claim_seen: false,
        lose_first_claim_ack: false,
        fail_first_claim_before_commit: true,
        calls: Vec::new(),
    };

    claim_current_owner_with_gateway(&config, Some(&authority), &context, &mut gateway)?;

    let claim_requests: Vec<_> = gateway
        .calls
        .iter()
        .filter(|(path, _, _)| path == CLAIM_PATH)
        .map(|(_, _, frame)| frame["payload"]["operation_id"].clone())
        .collect();
    assert_eq!(
        claim_requests,
        [
            Value::String(context.claim.operation_id.clone()),
            Value::String(context.claim.operation_id.clone())
        ]
    );
    assert_eq!(
        gateway
            .calls
            .iter()
            .map(|(path, _, _)| path.as_str())
            .collect::<Vec<_>>(),
        [READ_PATH, LOOKUP_PATH, CLAIM_PATH, LOOKUP_PATH, CLAIM_PATH]
    );
    Ok(())
}

#[test]
fn running_resume_requires_the_same_live_owner_and_historical_claim() -> Result<(), String> {
    let (_scratch, config, authority, context) = setup()?;
    let store =
        SqliteBranchStore::open(&context.branch_store_path).map_err(|error| error.to_string())?;
    let owner = owner_for(&config, &authority);
    let snapshot = store
        .snapshot_continuation_owner(
            &context.claim.operation_id,
            &serde_json::to_string(&owner).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
    store
        .transition_continuation_claim(
            &snapshot.operation_id,
            sts2_harness::BranchContinuationClaimState::OwnerSnapshotted,
            sts2_harness::BranchContinuationClaimState::Claimed,
        )
        .map_err(|error| error.to_string())?;
    store
        .transition_continuation_claim(
            &snapshot.operation_id,
            sts2_harness::BranchContinuationClaimState::Claimed,
            sts2_harness::BranchContinuationClaimState::BoundaryVerified,
        )
        .map_err(|error| error.to_string())?;
    let mut gateway = RecordingOwnerGateway {
        owner,
        state: "available",
        claim_seen: true,
        lose_first_claim_ack: false,
        fail_first_claim_before_commit: false,
        calls: Vec::new(),
    };

    claim_current_owner_with_gateway(&config, Some(&authority), &context, &mut gateway)?;

    assert_eq!(
        gateway
            .calls
            .iter()
            .map(|(path, _, _)| path.as_str())
            .collect::<Vec<_>>(),
        [READ_PATH, LOOKUP_PATH]
    );
    assert!(!gateway.calls.iter().any(|(path, _, _)| path == CLAIM_PATH));
    Ok(())
}
