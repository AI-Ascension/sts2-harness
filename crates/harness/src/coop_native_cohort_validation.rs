// SPDX-License-Identifier: MIT

fn validate_profile(envelope: &CoopNativeEnvelope) -> Result<(), CoopNativeCohortError> {
    let value = envelope.as_value();
    if value.get("protocol_version").and_then(serde_json::Value::as_str)
        != Some(COOP_NATIVE_PROTOCOL_VERSION)
        || value.get("schema_digest").and_then(serde_json::Value::as_str)
            != Some(COOP_NATIVE_SCHEMA_DIGEST)
    {
        return Err(CoopNativeCohortError::ProfileMismatch);
    }
    Ok(())
}

fn validate_ready_observation(observation: &CoopNativeObservation) -> Result<(), CoopNativeCohortError> {
    if !observation.host_digest_known() || observation.host_loading() || observation.host_divergent() {
        return Err(CoopNativeCohortError::NotReady);
    }
    for peer in observation.peers() {
        if !peer.connected() || peer.is_loading() || peer.is_divergent() || !peer.digest_known() {
            return Err(CoopNativeCohortError::NotReady);
        }
        if peer.checkpoint_id() != Some(observation.checkpoint_id())
            || peer.state_digest() != observation.state_digest()
            || peer.authority_id() != observation.authority_id()
        {
            return Err(CoopNativeCohortError::CheckpointMismatch);
        }
    }
    Ok(())
}

fn sole_local_peer(observation: &CoopNativeObservation) -> Option<&CoopNativePeerId> {
    let mut locals = observation
        .peers()
        .iter()
        .filter(|peer| peer.role() == CoopNativePeerRole::Local);
    let local = locals.next()?;
    locals.next().is_none().then_some(local.peer_token())
}

fn roster_of(observation: &CoopNativeObservation) -> BTreeSet<CoopNativePeerId> {
    observation
        .peers()
        .iter()
        .map(|peer| peer.peer_token().clone())
        .collect()
}

fn same_fence(left: &CoopNativeEnvelope, right: &CoopNativeEnvelope) -> bool {
    let left = left.header();
    let right = right.header();
    left.instance_id() == right.instance_id()
        && left.session_id() == right.session_id()
        && left.lease_id() == right.lease_id()
        && left.lease_epoch() == right.lease_epoch()
}

fn operation_for(
    route_peer: CoopNativePeerId,
    operation_id: CoopNativeOperationId,
    envelope: &CoopNativeEnvelope,
    expected_host_generation: u64,
) -> CoopNativeCohortOperation {
    let header = envelope.header();
    CoopNativeCohortOperation {
        route_peer,
        operation_id,
        instance_id: header.instance_id().as_str().to_owned(),
        session_id: header.session_id().as_str().to_owned(),
        lease_id: header.lease_id().as_str().to_owned(),
        lease_epoch: header.lease_epoch(),
        expected_host_generation,
    }
}

fn same_operation_fence(operation: &CoopNativeCohortOperation, envelope: &CoopNativeEnvelope) -> bool {
    let header = envelope.header();
    header.instance_id().as_str() == operation.instance_id
        && header.session_id().as_str() == operation.session_id
        && header.lease_id().as_str() == operation.lease_id
        && header.lease_epoch() == operation.lease_epoch
        && envelope.operation_id() == Some(operation.operation_id())
}
