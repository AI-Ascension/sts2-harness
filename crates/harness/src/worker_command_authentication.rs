// SPDX-License-Identifier: MIT

//! Authenticated request proof and transport-selected capabilities.

use crate::WorkerOwnerProof;
use crate::worker_handoff::{WorkerCommand, WorkerRequest};

/// An authorization capability is selected by the authenticated transport, never by the frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerCapability {
    Probe,
    Dispatch,
    Lookup,
    Acknowledge,
    SetControlMode,
}

impl WorkerCapability {
    pub const fn command(self) -> WorkerCommand {
        match self {
            Self::Probe => WorkerCommand::Probe,
            Self::Dispatch => WorkerCommand::Dispatch,
            Self::Lookup => WorkerCommand::Lookup,
            Self::Acknowledge => WorkerCommand::Acknowledge,
            Self::SetControlMode => WorkerCommand::SetControlMode,
        }
    }
}

/// Transport code fills this only after protected local peer authentication. It has no public
/// constructor and deliberately carries no boolean authentication claim.
///
/// External callers cannot turn a marker string into authenticated admission:
/// ```compile_fail,E0624
/// use sts2_harness::{WorkerOwnerProof, worker_handoff::{
///     AuthenticatedWorkerRequest, WorkerCapability, WorkerRequest,
/// }};
/// fn forge(request: WorkerRequest, proof: WorkerOwnerProof) {
///     let _ = AuthenticatedWorkerRequest::from_transport(
///         request, WorkerCapability::Dispatch, proof,
///     );
/// }
/// ```
pub struct AuthenticatedWorkerRequest {
    pub(in crate::worker_handoff) request: WorkerRequest,
    pub(in crate::worker_handoff) capability: WorkerCapability,
    pub(in crate::worker_handoff) owner_proof: WorkerOwnerProof,
}

impl AuthenticatedWorkerRequest {
    /// Marks a request as transport-authenticated after the caller has completed its protected
    /// peer and credential checks.  The owner proof is deliberately retained as an opaque value;
    /// this constructor does not authenticate a peer or inspect a credential.
    #[allow(dead_code)]
    pub(crate) fn from_transport(
        request: WorkerRequest,
        capability: WorkerCapability,
        owner_proof: WorkerOwnerProof,
    ) -> Self {
        Self {
            request,
            capability,
            owner_proof,
        }
    }

    /// Returns the validated request retained by the endpoint for response correlation.
    pub fn request(&self) -> &WorkerRequest {
        &self.request
    }
}
