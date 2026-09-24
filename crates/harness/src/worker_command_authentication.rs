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
/// External callers cannot turn a marker string into authenticated admission. The two fences below
/// are a pair and both are required.
///
/// This one pins the import surface the compile-fail fence depends on. It must **compile**, so it
/// turns red the moment one of these public paths is renamed or removed:
/// ```
/// use sts2_harness::{WorkerOwnerProof, worker_handoff::{
///     AuthenticatedWorkerRequest, WorkerCapability, WorkerRequest,
/// }};
/// fn pinned(request: WorkerRequest, proof: WorkerOwnerProof) {
///     let _: Option<&AuthenticatedWorkerRequest> = None;
///     let _ = (std::mem::size_of::<AuthenticatedWorkerRequest>(), request, proof);
/// }
/// fn pinned_capability() { let _ = WorkerCapability::Dispatch; }
/// ```
/// This one asserts the authority property, and only that property:
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
///
/// The `,E0624` annotation is parsed but **not** enforced by rustdoc: a `compile_fail` fence whose
/// snippet dies of `E0432`/`E0425` before it reaches the constructor still reports success, and an
/// unknown code such as `E9999` is accepted silently. So the annotation alone cannot protect this
/// property. The compiling fence above is what makes a missing path observable, leaving the
/// `compile_fail` fence to assert what it can actually assert. See `sts2-harness#481`.
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
