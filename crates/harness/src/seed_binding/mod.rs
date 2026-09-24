// SPDX-License-Identifier: MIT

//! Authored-workflow seed resolution and durable binding.
//!
//! An authored workflow may name an explicit seed or ask the harness to generate
//! one exactly once. This module resolves that request into a single effective
//! seed per logical admitted run, binds it to the run's instance, profile
//! baseline, lease and setup context, persists the effective seed before any
//! setup mutation, and refuses a setup that does not match. It is source-only:
//! no native host accepted the seed here. Native acceptance stays tracked by
//! `sts2-game-mod#79` (issue #103, AC3).

mod canonical;
mod resolve;

use std::fmt;

use serde::Serialize;

pub use canonical::{MAX_SEED_BYTES, canonicalize_seed};
pub use resolve::{
    OsSeedSource, RecordingTransport, ResolvedSeed, SeedRecord, SeedSource, SeedStore, SentStart,
    StartTransport, dispatch_start, resolve_seed,
};

/// Only supported persisted seed-binding version. Unknown semantics require a new
/// version and its compatibility note.
pub const SEED_BINDING_VERSION: &str = "ascension.seed-binding.v1";

/// Bounded, non-reflecting seed-binding failures. No supplied seed is echoed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SeedBindingError {
    /// The supplied or generated seed was empty.
    EmptySeed,
    /// The canonical seed exceeded [`MAX_SEED_BYTES`] UTF-8 bytes.
    SeedTooLarge,
    /// The seed contained a control character.
    SeedControlCharacter,
    /// The seed carried leading or trailing whitespace.
    SeedNotCanonical,
    /// The requested setup is not in the admitted vocabulary.
    UnsupportedSetup,
    /// The requested instance does not match the observed instance.
    InstanceMismatch,
    /// The requested profile baseline is stale against the observed baseline.
    BaselineMismatch,
    /// The requested lease is stale against the observed lease.
    LeaseMismatch,
    /// A persisted record or explicit seed conflicts with this request.
    ConfigurationConflict,
    /// The effective seed could not be durably persisted before mutation.
    PersistenceFailed,
    /// The store could not be read.
    StoreUnavailable,
    /// The operating-system randomness source was unavailable.
    RandomnessUnavailable,
}

impl fmt::Display for SeedBindingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "seed binding error: {self:?}")
    }
}

impl std::error::Error for SeedBindingError {}

/// How an authored workflow names its seed for one logical run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SeedMode {
    /// Caller-supplied seed, normalized to its canonical form.
    Explicit(String),
    /// Draw exactly one seed the first time the run resolves.
    GenerateOnce,
}

/// The authored setup facts a resolved seed must bind to.
///
/// The `observed_*` fields are the live environment's current values; a
/// disagreement with the requested binding is refused before any draw or
/// mutation.
#[derive(Clone, Debug)]
pub struct SetupRequest {
    /// The logical operation identity this seed belongs to.
    pub operation_id: String,
    /// The game instance the run intends to use.
    pub instance_id: String,
    /// The instance the environment currently reports.
    pub observed_instance_id: String,
    /// The profile baseline digest the run intends to bind.
    pub baseline_digest: String,
    /// The profile baseline digest the environment currently reports.
    pub observed_baseline_digest: String,
    /// The lease the run intends to hold.
    pub lease_id: String,
    /// The lease the environment currently reports.
    pub observed_lease_id: String,
    /// The setup context identity the run intends to use.
    pub setup: String,
    /// The admitted setup context vocabulary.
    pub supported_setups: Vec<String>,
    /// The requested seed mode.
    pub mode: SeedMode,
}

impl SetupRequest {
    /// Reject a setup that cannot bind this run, before any draw or mutation.
    ///
    /// The first failing property is reported, in instance, baseline, lease, then
    /// setup order.
    ///
    /// # Errors
    ///
    /// Returns the specific mismatch when the request does not match the observed
    /// environment or the admitted setup vocabulary.
    pub fn validate(&self) -> Result<(), SeedBindingError> {
        if self.instance_id != self.observed_instance_id {
            return Err(SeedBindingError::InstanceMismatch);
        }
        if self.baseline_digest != self.observed_baseline_digest {
            return Err(SeedBindingError::BaselineMismatch);
        }
        if self.lease_id != self.observed_lease_id {
            return Err(SeedBindingError::LeaseMismatch);
        }
        if !self.supported_setups.contains(&self.setup) {
            return Err(SeedBindingError::UnsupportedSetup);
        }
        Ok(())
    }
}
