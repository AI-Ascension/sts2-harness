// SPDX-License-Identifier: MIT

impl CoopNativeCohort {
    pub fn validate_recovery(
        &self,
        operation: &CoopNativeCohortOperation,
        route_peer: &CoopNativePeerId,
        request: &CoopNativeEnvelope,
    ) -> Result<(), CoopNativeCohortError> {
        let route = self
            .routes
            .get(route_peer)
            .ok_or(CoopNativeCohortError::RecoveryNotOriginalRoute)?;
        if operation.route_peer() != route_peer
            || !same_operation_fence(operation, request)
            || !same_fence(route.observation(), request)
        {
            return Err(CoopNativeCohortError::RecoveryNotOriginalRoute);
        }
        match request.kind() {
            CoopNativeKind::RejoinRequest => {
                let recovery = request
                    .rejoin_request()
                    .ok_or(CoopNativeCohortError::RecoveryNotRequest)?;
                if recovery.operation_id() != operation.operation_id()
                    || recovery.actor_peer() != route_peer
                {
                    return Err(CoopNativeCohortError::OperationMismatch);
                }
            }
            CoopNativeKind::RecoveryResponse => {
                let recovery = request
                    .recovery_response()
                    .ok_or(CoopNativeCohortError::RecoveryNotRequest)?;
                if recovery.status().is_some() || recovery.operation_id() != operation.operation_id() {
                    return Err(CoopNativeCohortError::OperationMismatch);
                }
            }
            _ => return Err(CoopNativeCohortError::RecoveryNotRequest),
        }
        Ok(())
    }
}
