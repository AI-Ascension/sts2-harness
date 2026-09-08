// SPDX-License-Identifier: MIT

#[path = "types_worker_completion.rs"]
mod completion;
#[path = "types_worker_control.rs"]
mod control;
#[path = "types_worker_identity.rs"]
mod identity;

pub use completion::{
    StoredWorkerHandoff, WorkerCompletionStatus, WorkerHandoffState, WorkerLookup,
    WorkerReservationState, WorkerTerminalReceipt,
};
pub use control::{WorkerControlMode, WorkerControlRequest, WorkerControlState, WorkerOwnerProof};
pub use identity::{
    WORKER_EMPTY_PARAMETERS_DIGEST, WORKER_HANDOFF_CONTRACT, WORKER_HANDOFF_SCHEMA_DIGEST,
    WORKER_MAX_ATTEMPT_NUMBER, WorkerAdmissionContext, WorkerBoot, WorkerTuple,
};

pub(crate) use completion::worker_terminal_from_completion;
pub(crate) use identity::{valid_worker_existing_id, valid_worker_identity, valid_worker_uuid4};

use std::fmt;
use std::sync::{Arc, Weak};

/// The one-time authorization produced only by a fresh worker-handoff insert.
///
/// A permit deliberately has no public constructor, clone, serialization, or identity accessor.
/// It is consumed by [`ExecutionStore::mark_worker_handoff_running`](super::ExecutionStore::mark_worker_handoff_running)
/// so a duplicate or reopened handoff can never be turned back into execution authority.
pub struct WorkerExecutionPermit {
    handoff_id: String,
    owner: Weak<()>,
}

impl WorkerExecutionPermit {
    pub(super) fn new(handoff_id: String, owner: &Arc<()>) -> Self {
        Self {
            handoff_id,
            owner: Arc::downgrade(owner),
        }
    }

    pub(super) fn belongs_to(&self, owner: &Arc<()>) -> bool {
        self.owner.ptr_eq(&Arc::downgrade(owner))
    }

    pub(super) fn handoff_id(self) -> String {
        self.handoff_id
    }
}

impl fmt::Debug for WorkerExecutionPermit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("WorkerExecutionPermit(REDACTED)")
    }
}

/// The atomic result of worker-handoff admission.
///
/// Only [`Acquired`](Self::Acquired) carries a non-replayable permit. A duplicate is a read-only
/// observation of the retained row and cannot authorize a new execution.
pub enum WorkerAdmissionOutcome {
    Acquired {
        handoff: StoredWorkerHandoff,
        permit: WorkerExecutionPermit,
    },
    Duplicate(StoredWorkerHandoff),
}

impl WorkerAdmissionOutcome {
    pub fn handoff(&self) -> &StoredWorkerHandoff {
        match self {
            Self::Acquired { handoff, .. } | Self::Duplicate(handoff) => handoff,
        }
    }

    pub fn into_acquired(self) -> Option<(StoredWorkerHandoff, WorkerExecutionPermit)> {
        match self {
            Self::Acquired { handoff, permit } => Some((handoff, permit)),
            Self::Duplicate(_) => None,
        }
    }
}
