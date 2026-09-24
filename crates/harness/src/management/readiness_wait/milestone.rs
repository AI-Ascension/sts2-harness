// SPDX-License-Identifier: MIT

//! The bounded readiness-milestone vocabulary.
//!
//! Each milestone is a specific authoritative state reported by the owning
//! surface, ordered from "a process exists" to "gameplay is actionable". A wait
//! settles only when an observation reports a milestone that reaches its target;
//! the milestone is never derived from elapsed time or from a listening port.

use serde::{Deserialize, Serialize};

/// An authoritative gameplay-readiness milestone.
///
/// The variants are the only states a target may name. Labels are stable and
/// reported verbatim, so a milestone never changes meaning when the vocabulary
/// gains a member.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessMilestone {
    /// The game process was born for the current generation.
    Booted,
    /// The mod adapter authenticated and reported a compatible contract.
    AdapterCompatible,
    /// The requested instance holds its current lease.
    LeaseInstalled,
    /// Authored setup settled, so actions may be admitted.
    SetupAvailable,
    /// Gameplay state is actionable, not merely reachable over a port.
    Actionable,
}

impl ReadinessMilestone {
    /// Returns the stable lowercase label of this milestone.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Booted => "booted",
            Self::AdapterCompatible => "adapter_compatible",
            Self::LeaseInstalled => "lease_installed",
            Self::SetupAvailable => "setup_available",
            Self::Actionable => "actionable",
        }
    }

    /// Returns this milestone's position in the ordered vocabulary.
    #[must_use]
    pub const fn rank(self) -> u8 {
        match self {
            Self::Booted => 0,
            Self::AdapterCompatible => 1,
            Self::LeaseInstalled => 2,
            Self::SetupAvailable => 3,
            Self::Actionable => 4,
        }
    }

    /// Whether an observed milestone is strong enough to meet `target`.
    #[must_use]
    pub const fn reaches(self, target: Self) -> bool {
        self.rank() >= target.rank()
    }
}
