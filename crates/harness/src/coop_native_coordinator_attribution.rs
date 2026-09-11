// SPDX-License-Identifier: MIT

struct OperationAttribution {
    expected_host_generation: Option<u64>,
    scheduled_actor_peer: super::identity::CoopNativePeerId,
    instance_id: CoopNativeInstanceId,
    session_id: super::identity::CoopNativeSessionId,
    lease_id: CoopNativeLeaseId,
    lease_epoch: u64,
}

impl OperationAttribution {
    fn from_request(envelope: &CoopNativeEnvelope) -> Result<Self, CoopNativeCoordinatorError> {
        Ok(Self {
            expected_host_generation: envelope.expected_host_generation(),
            scheduled_actor_peer: envelope
                .actor_peer()
                .cloned()
                .ok_or(CoopNativeCoordinatorError::Serialization)?,
            instance_id: envelope.header().instance_id().clone(),
            session_id: envelope.header().session_id().clone(),
            lease_id: envelope.header().lease_id().clone(),
            lease_epoch: envelope.header().lease_epoch(),
        })
    }
}

impl<P: CoopNativePort> CoopNativeCoordinator<P> {
    fn validate_response_fence(
        &self,
        envelope: &CoopNativeEnvelope,
        entry: &OperationEntry,
    ) -> Result<(), CoopNativeCoordinatorError> {
        if envelope.header().instance_id() != &entry.attribution.instance_id
            || envelope.header().session_id() != &entry.attribution.session_id
            || envelope.header().lease_id() != &entry.attribution.lease_id
            || envelope.header().lease_epoch() != entry.attribution.lease_epoch
        {
            return Err(CoopNativeCoordinatorError::RouteFenceMismatch);
        }
        Ok(())
    }

    fn validate_observation_peer(
        &self,
        envelope: &CoopNativeEnvelope,
        operation: Option<&CoopNativeOperationId>,
    ) -> Result<(), CoopNativeCoordinatorError> {
        let Some(observation) = envelope.observation() else {
            return Ok(());
        };
        let local_peer = observation
            .local_peer_token()
            .ok_or(CoopNativeCoordinatorError::CanonicalPeerMismatch)?;
        let scheduled_peer = operation
            .and_then(|operation_id| self.operations.get(operation_id))
            .map(|entry| &entry.attribution.scheduled_actor_peer)
            .or_else(|| envelope.actor_peer());
        if scheduled_peer.is_some_and(|peer| peer != local_peer) {
            return Err(CoopNativeCoordinatorError::CanonicalPeerMismatch);
        }
        Ok(())
    }
}
