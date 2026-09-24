// SPDX-License-Identifier: MIT

//! Identity-bound gameplay-readiness waits for authored workflows (issue #96).
//!
//! The launch-acknowledgement/readiness split in
//! [`crate::management::lifecycle_readiness`] proves that a process launch is not
//! gameplay readiness, and binds readiness evidence to one instance and
//! authority epoch. This module adds what an authored workflow actually needs in
//! order to *wait*: a versioned milestone target with a bounded deadline, and a
//! per-generation wait that only a fresh, correctly bound authoritative
//! observation can settle.
//!
//! The milestone is reported by the observing owner; it is never inferred from
//! elapsed time or from a listening port. A restart invalidates prior readiness,
//! so a wait bound to a superseded generation refuses stale observations and
//! requires fresh proof.

mod error;
mod milestone;
mod target;
mod wait;

pub use error::ReadinessWaitError;
pub use milestone::ReadinessMilestone;
pub use target::{READINESS_CONTRACT_VERSION, ReadinessTarget};
pub use wait::{MilestoneObservation, ReadinessProgress, ReadinessTerminal, ReadinessWait};
