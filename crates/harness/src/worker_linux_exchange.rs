// SPDX-License-Identifier: MIT

//! Native authenticated transport-to-admission bridge. The connection and its
//! held peer identity remain owned until the correlated response is written.

use crate::execution::WorkerOwnerProof;
use crate::worker_frame_io::ConnectionDeadline;
use crate::worker_handoff::{
    AuthenticatedWorkerRequest, WorkerCapability, WorkerReply, WorkerRequest,
};
use crate::worker_local_linux::{
    AuthenticatedWorkerConnection, LinuxTransportError, LinuxWorkerListener,
};

/// One authenticated request and its original native connection. Neither can
/// be replaced by callers or detached into independently reusable authority.
pub struct LinuxWorkerExchange {
    connection: AuthenticatedWorkerConnection,
    authenticated: AuthenticatedWorkerRequest,
}

impl LinuxWorkerExchange {
    /// Accept using the configured OS peer and credential policy, then decode
    /// the received frame. `capability` must come from local endpoint policy,
    /// never from request fields. A mismatched command is rejected here.
    pub async fn accept(
        listener: &LinuxWorkerListener,
        deadline: ConnectionDeadline,
        capability: WorkerCapability,
    ) -> Result<Self, LinuxTransportError> {
        let mut connection = listener.accept_authenticated(deadline).await?;
        let bytes = connection.read_request_bytes().await?;
        let request = WorkerRequest::decode(&bytes).map_err(|_| LinuxTransportError::Framing)?;
        if request.command() != capability.command() {
            return Err(LinuxTransportError::Credential);
        }
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
