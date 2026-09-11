// SPDX-License-Identifier: MIT

#[test]
fn cohort_recovery_requires_the_original_route_fence_and_operation() -> Result<(), Box<dyn Error>> {
    let cohort = cohort()?;
    let host = CoopNativePeerId::new("peer:host1")?;
    let scheduled = cohort.schedule_local_action(
        &host,
        &action_fixture("host", "peer:host1", "op:native:cohort-recovery")?,
    )?;
    let recovery = recovery_fixture("host", "op:native:cohort-recovery")?;
    assert_eq!(cohort.validate_recovery(&scheduled, &host, &recovery), Ok(()));
    assert_eq!(
        cohort.validate_recovery(
            &scheduled,
            &CoopNativePeerId::new("peer:client1")?,
            &recovery,
        ),
        Err(CoopNativeCohortError::RecoveryNotOriginalRoute)
    );
    Ok(())
}

#[test]
fn cohort_settlement_requires_host_evidence_and_all_roster_convergence() -> Result<(), Box<dyn Error>> {
    let cohort = cohort()?;
    let host = CoopNativePeerId::new("peer:host1")?;
    let scheduled = cohort.schedule_local_action(
        &host,
        &action_fixture("host", "peer:host1", "op:native:cohort-settlement")?,
    )?;
    let mut response: Value = serde_json::from_slice(golden("local-action-settled-response"))?;
    response["instance_id"] = json!("instance:native-host");
    response["session_id"] = json!("session:native-host");
    response["lease_id"] = json!("lease:native-host");
    response["lease_epoch"] = json!(7);
    response["operation_id"] = json!("op:native:cohort-settlement");
    response["receipt"]["operation_id"] = json!("op:native:cohort-settlement");
    response["effect"]["operation_id"] = json!("op:native:cohort-settlement");
    let checksum = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    response["observation"]["checksum_status"] = json!("matched");
    response["observation"]["native_checksum"] = json!(checksum);
    for peer in response["observation"]["peers"].as_array_mut().ok_or("peers")? {
        peer["checksum_status"] = json!("matched");
    }
    let settled = CoopNativeEnvelope::parse_response(&serde_json::to_vec(&response)?)?;

    let mut host_observation: Value = serde_json::from_slice(golden("observation-response"))?;
    host_observation["instance_id"] = json!("instance:native-host");
    host_observation["session_id"] = json!("session:native-host");
    host_observation["lease_id"] = json!("lease:native-host");
    host_observation["lease_epoch"] = json!(7);
    host_observation["observation"] = response["observation"].clone();
    let mut client_observation = host_observation.clone();
    client_observation["instance_id"] = json!("instance:native-client");
    client_observation["session_id"] = json!("session:native-client");
    client_observation["lease_id"] = json!("lease:native-client");
    client_observation["observation"]["peers"][0]["peer_token"] = json!("peer:client1");
    client_observation["observation"]["peers"][1]["peer_token"] = json!("peer:host1");
    client_observation["observation"]["peers"][0]["role"] = json!("local");
    client_observation["observation"]["peers"][1]["role"] = json!("ally");
    let refreshed = vec![
        CoopNativeEnvelope::parse_response(&serde_json::to_vec(&host_observation)?)?,
        CoopNativeEnvelope::parse_response(&serde_json::to_vec(&client_observation)?)?,
    ];
    assert_eq!(cohort.validate_settlement(&scheduled, &host, &settled, &refreshed), Ok(()));

    let mut loading_response = response.clone();
    loading_response["observation"]["host_loading"] = json!(true);
    let loading = CoopNativeEnvelope::parse_response(&serde_json::to_vec(&loading_response)?)?;
    assert_eq!(
        cohort.validate_settlement(&scheduled, &host, &loading, &refreshed),
        Err(CoopNativeCohortError::SettlementEvidence)
    );

    let only_host = vec![refreshed[0].clone()];
    assert_eq!(
        cohort.validate_settlement(&scheduled, &host, &settled, &only_host),
        Err(CoopNativeCohortError::RosterNotConverged)
    );
    Ok(())
}
