// SPDX-License-Identifier: MIT

//! Source-only admission of an alternative gameplay fork from a verified seeded replay prefix.
//!
//! A fork restarts one recorded seeded run, replays exactly the settled prefix up to a selected
//! decision boundary with zero provider calls, then hands that boundary to a fresh child branch
//! that continues with a different next decision. No game is launched and no provider is invoked
//! here: the replay port, the destination lease and the child process belong to the gateway, and
//! this module fixes the effect-free admission contract and its failure fixtures.
//!
//! The owner API is deliberately small:
//!
//! - select and bind: [`PrefixBoundary`] and [`ForkBinding`] name the exact fork point;
//! - admit: [`admit_prefix_fork`] returns a [`PrefixForkPlan`] or a [`PrefixForkRefusal`];
//! - siblings: [`SiblingSet`] keeps one source prefix with distinct child identities;
//! - hand off: [`admit_handoff`], [`next_handoff_stage`] and [`reconcile_lost_handoff`].

mod admission;
mod binding;
mod boundary;
mod error;
mod handoff;
mod sibling;

pub use admission::{PrefixForkPlan, PrefixForkRequest, admit_prefix_fork};
pub use binding::{ForkBinding, ForkBindingError, ForkBindingMismatch};
pub use boundary::{
    LegalBinding, MAX_FORK_ORDINAL, MAX_PREFIX_RECEIPTS, PrefixBoundary, PrefixBoundaryError,
    ReplayObservation,
};
pub use error::PrefixForkRefusal;
pub use handoff::{
    HandoffError, HandoffMode, HandoffReconciliation, HandoffStage, admit_handoff,
    next_handoff_stage, reconcile_lost_handoff,
};
pub use sibling::{MAX_SIBLINGS, SiblingError, SiblingFork, SiblingSet};

/// Maximum bytes of a fork experiment, continuation, occurrence or action label.
pub const MAX_FORK_LABEL_BYTES: usize = 256;

/// Reports whether a bounded label is non-empty, within its byte bound and NUL-free.
#[must_use]
pub(crate) fn label_ok(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_FORK_LABEL_BYTES && !value.contains('\0')
}
