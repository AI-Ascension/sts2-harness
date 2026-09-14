// SPDX-License-Identifier: MIT

//! Opt-in owner persistence protocol. No native executor or runtime admission is provided.

mod migration;
mod owner;
mod ports;
mod reconcile;
mod recovery;
mod start;
#[cfg(all(test, unix))]
mod tests;
pub(crate) mod types;
mod validation;

pub use owner::{InFlight, LifecycleOwner, StartOutcome};
pub use ports::{
    AuthorityGuard, ClaimKind, EffectCompletion, EffectHandle, EffectPort, LifecycleAuthorityPort,
    OwnerClaim, SendPermit,
};
pub use types::{
    AuthorityVector, InvocationManifest, JournalConfig, LifecycleEntry, LifecycleError,
    LifecyclePhase, MAX_INPUT_BYTES, MAX_LIFECYCLE_ENTRIES, NativeIdentity,
};
