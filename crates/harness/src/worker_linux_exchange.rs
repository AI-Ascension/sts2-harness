// SPDX-License-Identifier: MIT

//! Native authenticated transport-to-admission bridge. The connection and its
//! held peer identity remain owned until the correlated response is written.

use crate::execution::WorkerOwnerProof;
use crate::worker_frame_io::ConnectionDeadline;
use crate::worker_handoff::{
    AuthenticatedWorkerRequest, WorkerCapability, WorkerCommand, WorkerReply, WorkerRequest,
};
use crate::worker_local_linux::{
    AuthenticatedWorkerConnection, LinuxTransportError, LinuxWorkerListener,
};

/// Approved local endpoint policy. This type is never deserialized from a
/// request. Even the owner policy still requires the configured native peer
/// and protected credential before any capability is selected.
#[derive(Clone, Copy)]
pub enum WorkerEndpointPolicy {
    Only(WorkerCapability),
    ReadOnly,
    WatchdogOwner,
}

impl WorkerEndpointPolicy {
    fn capability_for(
        self,
        command: WorkerCommand,
    ) -> Result<WorkerCapability, LinuxTransportError> {
        let capability = match command {
            WorkerCommand::Probe => WorkerCapability::Probe,
            WorkerCommand::Dispatch => WorkerCapability::Dispatch,
            WorkerCommand::Lookup => WorkerCapability::Lookup,
            WorkerCommand::Acknowledge => WorkerCapability::Acknowledge,
            WorkerCommand::SetControlMode => WorkerCapability::SetControlMode,
        };
        let permitted = match self {
            Self::Only(allowed) => allowed == capability,
            Self::ReadOnly => matches!(
                capability,
                WorkerCapability::Probe | WorkerCapability::Lookup
            ),
            Self::WatchdogOwner => true,
        };
        if !permitted {
            return Err(LinuxTransportError::Credential);
        }
        Ok(capability)
    }
}

/// One authenticated request and its original native connection. Neither can
/// be replaced by callers or detached into independently reusable authority.
pub struct LinuxWorkerExchange<'a> {
    connection: AuthenticatedWorkerConnection<'a>,
    authenticated: AuthenticatedWorkerRequest,
}

impl<'a> LinuxWorkerExchange<'a> {
    /// Accept using the configured OS peer and credential policy, then decode
    /// the received frame. `capability` must come from local endpoint policy,
    /// never from request fields. A mismatched command is rejected here.
    pub async fn accept(
        listener: &'a LinuxWorkerListener,
        deadline: ConnectionDeadline,
        capability: WorkerCapability,
    ) -> Result<Self, LinuxTransportError> {
        Self::accept_with_policy(listener, deadline, WorkerEndpointPolicy::Only(capability)).await
    }

    /// Receive one request under a locally approved capability set. Reading a
    /// command selects only within that set; it cannot enlarge endpoint rights.
    pub async fn accept_with_policy(
        listener: &'a LinuxWorkerListener,
        deadline: ConnectionDeadline,
        policy: WorkerEndpointPolicy,
    ) -> Result<Self, LinuxTransportError> {
        let mut connection = listener.accept_authenticated(deadline).await?;
        let bytes = connection.read_request_bytes().await?;
        let request = WorkerRequest::decode(&bytes).map_err(|_| LinuxTransportError::Framing)?;
        let capability = policy.capability_for(request.command())?;
        // Construction is reachable only after native authentication; the
        // peer witness stays inside `connection`, not in serialized claims.
        let owner_proof = WorkerOwnerProof::new("linux-native-worker-peer-v1")
            .map_err(|_| LinuxTransportError::Configuration)?;
        Ok(Self {
            connection,
            authenticated: AuthenticatedWorkerRequest::from_transport(
                request,
                capability,
                owner_proof,
            ),
        })
    }

    /// Borrow admission input while retaining the authenticated connection.
    pub fn request(&self) -> &AuthenticatedWorkerRequest {
        &self.authenticated
    }

    /// Consume the exchange even when encoding or writing fails. Response
    /// correlation is derived from the original decoded request only.
    pub async fn write_reply(
        mut self,
        worker_boot_id: &str,
        reply: WorkerReply,
    ) -> Result<(), LinuxTransportError> {
        let bytes = self
            .authenticated
            .request()
            .encode_response(worker_boot_id, reply)
            .map_err(|_| LinuxTransportError::Framing)?;
        self.connection.write_response_bytes(&bytes).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_policy_limits_every_command_without_request_granted_rights() {
        let capabilities = [
            WorkerCapability::Probe,
            WorkerCapability::Dispatch,
            WorkerCapability::Lookup,
            WorkerCapability::Acknowledge,
            WorkerCapability::SetControlMode,
        ];
        for capability in capabilities {
            let command = capability.command();
            assert_eq!(
                WorkerEndpointPolicy::WatchdogOwner.capability_for(command),
                Ok(capability)
            );
            let read_only = WorkerEndpointPolicy::ReadOnly.capability_for(command);
            assert_eq!(
                read_only.is_ok(),
                matches!(
                    capability,
                    WorkerCapability::Probe | WorkerCapability::Lookup
                )
            );
            for allowed in capabilities {
                assert_eq!(
                    WorkerEndpointPolicy::Only(allowed)
                        .capability_for(command)
                        .is_ok(),
                    allowed == capability
                );
            }
        }
    }
}
