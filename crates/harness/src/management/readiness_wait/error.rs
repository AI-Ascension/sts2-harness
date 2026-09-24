// SPDX-License-Identifier: MIT

//! Refusals a versioned readiness target or wait can report.

/// A bounded, distinguishable refusal from admitting a readiness target or
/// waiting for its milestone.
///
/// The variants are the fixed refusal vocabulary a workflow router branches on:
/// an unsupported target is refused before any work starts ([`Self::Incompatible`]),
/// a missing or foreign proof never settles the wait, and exhaustion is reported
/// separately from an owner denial.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadinessWaitError {
    /// The target's contract version is not the one this harness implements.
    Incompatible,
    /// The target was structurally unusable (zero deadline or zero attempt bound).
    InvalidTarget,
    /// The wait already settled, was cancelled or invalidated by a restart.
    Settled,
    /// The evidence named a different instance or authority epoch.
    ForeignReadiness,
    /// The evidence was captured under a superseded process generation.
    StaleReadiness,
    /// The bounded deadline or attempt budget was exhausted.
    Timeout,
    /// The owning lifecycle surface explicitly denied the milestone.
    Denied,
}

impl std::fmt::Display for ReadinessWaitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::Incompatible => "readiness target contract version is unsupported",
            Self::InvalidTarget => "readiness target is structurally invalid",
            Self::Settled => "readiness wait is already settled",
            Self::ForeignReadiness => "readiness evidence is for another instance or epoch",
            Self::StaleReadiness => "readiness evidence is stale for this generation",
            Self::Timeout => "readiness wait exceeded its bounded deadline or attempt budget",
            Self::Denied => "readiness was denied by the owning lifecycle surface",
        };
        formatter.write_str(label)
    }
}

impl std::error::Error for ReadinessWaitError {}
