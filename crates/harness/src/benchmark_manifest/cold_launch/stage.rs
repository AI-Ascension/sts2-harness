// SPDX-License-Identifier: MIT

//! The bounded cold-launch stage vocabulary and its stable labels.
//!
//! Stages are reported verbatim, so each stage has one lowercase label that never changes when
//! the transition machine gains a member.

use serde::Serialize;

/// Maximum bytes of a cold-launch trial key.
pub const MAX_COLD_TRIAL_KEY_BYTES: usize = 256;

/// Stages one cold-launch trial passes through.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ColdLaunchStage {
    /// Admitted against a validated baseline; nothing allocated yet.
    Admitted,
    /// A fresh writable destination is exclusively leased to this trial.
    DestinationReserved,
    /// A separate writable clone was provisioned from the immutable baseline.
    Provisioned,
    /// A new native game process was born and attested.
    Launched,
    /// The new process proved readiness for its own birth generation.
    Ready,
    /// Authored setup settled before any action was admitted.
    SetupSettled,
    /// The seeded run is executing.
    Running,
    /// The process was stopped; evidence retained.
    Stopped,
    /// The destination was cleaned and may be released.
    Cleaned,
    /// Cleanup failed; distinct from the gameplay outcome and blocks destination reuse.
    CleanupFailed,
    /// The destination is uncertain and is never reused.
    Quarantined,
}

impl ColdLaunchStage {
    /// Returns the stable lowercase label of this stage.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Admitted => "admitted",
            Self::DestinationReserved => "destination_reserved",
            Self::Provisioned => "provisioned",
            Self::Launched => "launched",
            Self::Ready => "ready",
            Self::SetupSettled => "setup_settled",
            Self::Running => "running",
            Self::Stopped => "stopped",
            Self::Cleaned => "cleaned",
            Self::CleanupFailed => "cleanup_failed",
            Self::Quarantined => "quarantined",
        }
    }
}
