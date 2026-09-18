// SPDX-License-Identifier: MIT

//! Opt-in owner persistence protocol and a receipt-bound, cancellable Exo process effect.
//!
//! Runtime admission remains responsible for constructing the owner context and selecting the
//! explicit v2 bridge profile; the source descriptor stays unverified until that composition runs.

mod bridge_v2;
#[cfg(test)]
mod bridge_v2_tests;
mod completed_replay;
mod migration;
mod owner;
mod ports;
mod process_effect;
mod process_reap;
mod reconcile;
mod recovery;
mod runtime_transport;
mod start;
mod store_binding;
#[cfg(all(test, unix))]
mod tests;
pub(crate) mod types;
mod validation;

pub use bridge_v2::{
    EXO_LIFECYCLE_WIRE_V2, ExoLifecycleResponse, LifecycleOutcome, parse_lifecycle_response,
};
pub use owner::{InFlight, LifecycleOwner, StartOutcome};
pub use ports::{
    AuthorityGuard, ClaimKind, EffectCompletion, EffectHandle, EffectPort, LifecycleAuthorityPort,
    OwnerClaim, SendPermit,
};
pub use process_effect::{LifecycleProcessEffect, LifecycleProcessHandle};
pub use runtime_transport::{ExoLifecycleRuntimeTransport, LifecycleManifestFactory};
pub use types::{
    AuthorityVector, InvocationManifest, JournalConfig, LifecycleEntry, LifecycleError,
    LifecyclePhase, MAX_INPUT_BYTES, MAX_LIFECYCLE_ENTRIES, NativeIdentity,
};
