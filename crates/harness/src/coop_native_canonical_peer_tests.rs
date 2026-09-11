// SPDX-License-Identifier: MIT

#[test]
fn returned_observations_require_the_scheduled_canonical_peer() -> Result<(), Box<dyn Error>> {
    let request = parse_request("local-action-settled-request")?;
    let operation = request.operation_id().cloned().ok_or("operation missing")?;
    let mut response: Value = serde_json::from_slice(golden("local-action-settled-response"))?;
    response["observation"]["peers"][0]["peer_token"] = json!("peer:client1");
    response["observation"]["peers"][1]["peer_token"] = json!("peer:host1");
    let response = CoopNativeEnvelope::parse_response(&serde_json::to_vec(&response)?)?;
    let mut coordinator = CoopNativeCoordinator::new(CountingPort::default(), lineage()?)?;

    coordinator.consume(request)?;
    assert_eq!(
        coordinator.consume(response),
        Err(CoopNativeCoordinatorError::CanonicalPeerMismatch)
    );
    assert_eq!(
        coordinator.operation_state(&operation),
        Some(CoopNativeOperationState::Requested)
    );
    assert_eq!(coordinator.port().calls, 1);
    assert_eq!(coordinator.records().len(), 1);
    Ok(())
}

#[test]
fn catalog_observation_cannot_attribute_a_different_actor_than_its_local_peer(
) -> Result<(), Box<dyn Error>> {
    let mut response: Value = serde_json::from_slice(golden("legal-catalog-response"))?;
    response["actor_peer"] = json!("peer:client1");
    response["catalog"]["actor_peer"] = json!("peer:client1");
    response["catalog"]["votes"][0]["voter_peer"] = json!("peer:client1");
    let response = CoopNativeEnvelope::parse_response(&serde_json::to_vec(&response)?)?;
    let mut coordinator = CoopNativeCoordinator::new(CountingPort::default(), lineage()?)?;

    assert_eq!(
        coordinator.consume(response),
        Err(CoopNativeCoordinatorError::CanonicalPeerMismatch)
    );
    assert_eq!(coordinator.port().calls, 0);
    assert!(coordinator.records().is_empty());
    Ok(())
}

#[test]
fn response_route_fence_mismatches_preserve_the_original_unknown_operation(
) -> Result<(), Box<dyn Error>> {
    let request = parse_request("local-action-unknown-request")?;
    let operation = request.operation_id().cloned().ok_or("operation missing")?;
    let unknown = parse_response("local-action-unknown-response")?;
    let original: Value = serde_json::from_slice(golden("local-action-recovered-response"))?;

    for (field, replacement) in [
        ("instance_id", json!("instance:native-other")),
        ("session_id", json!("session:native-other")),
        ("lease_id", json!("lease:native-other")),
        ("lease_epoch", json!(8)),
    ] {
        let mut response = original.clone();
        response[field] = replacement;
        let response = CoopNativeEnvelope::parse_response(&serde_json::to_vec(&response)?)?;
        let mut coordinator = CoopNativeCoordinator::new(CountingPort::default(), lineage()?)?;

        coordinator.consume(request.clone())?;
        coordinator.consume(unknown.clone())?;
        assert_eq!(
            coordinator.consume(response),
            Err(CoopNativeCoordinatorError::RouteFenceMismatch),
            "{field}"
        );
        assert_eq!(
            coordinator.operation_state(&operation),
            Some(CoopNativeOperationState::Unknown)
        );
        assert_eq!(coordinator.port().calls, 2);
        assert_eq!(coordinator.records().len(), 2);
    }
    Ok(())
}

#[test]
fn canonical_peer_attribution_does_not_modify_the_frozen_v1_artifact(
) -> Result<(), Box<dyn Error>> {
    assert_eq!(
        crate::sha256_hex(coop_native_schema_bytes()),
        COOP_NATIVE_SCHEMA_DIGEST
    );
    assert_eq!(verify_coop_native_artifact(), Ok(()));
    assert_eq!(
        verify_coop_native_candidate_artifact()?.status(),
        CoopNativeArtifactStatus::AcceptedComponent
    );
    Ok(())
}
