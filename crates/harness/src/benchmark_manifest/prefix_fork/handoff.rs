// SPDX-License-Identifier: MIT

//! The replay-to-child handoff boundary and its lost-reply reconciliation.

use std::fmt;

/// How the destination a forked child continues from is expected to be reached.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HandoffMode {
    /// The destination is retained and must be revalidated before the child continues.
    Retained,
    /// The destination was destroyed; no continuation is possible.
    Destroyed,
    /// The destination was lost; no continuation is possible.
    Lost,
    /// The destination changed identity; no continuation is possible.
    Changed,
}

impl HandoffMode {
    /// Returns the stable lowercase label of this mode.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Retained => "retained",
            Self::Destroyed => "destroyed",
            Self::Lost => "lost",
            Self::Changed => "changed",
        }
    }
}

/// Stages one prefix fork passes through across the handoff.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HandoffStage {
    /// A settled boundary was selected; nothing replayed yet.
    PrefixSelected,
    /// The prefix replayed exactly with zero provider calls.
    PrefixReplayed,
    /// A handoff to the destination was intended.
    HandoffIntended,
    /// The child was admitted from the boundary.
    ChildAdmitted,
    /// The child is running with isolated context.
    ChildRunning,
}

impl HandoffStage {
    /// Returns the stable lowercase label of this stage.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::PrefixSelected => "prefix_selected",
            Self::PrefixReplayed => "prefix_replayed",
            Self::HandoffIntended => "handoff_intended",
            Self::ChildAdmitted => "child_admitted",
            Self::ChildRunning => "child_running",
        }
    }
}

/// Refusal reasons for a handoff.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HandoffError {
    /// A retained destination was not revalidated before continuation.
    StaleRetainedTarget,
    /// The destination cannot be reached in this mode.
    UnavailableTarget(HandoffMode),
    /// The requested stage transition is not legal from the current stage.
    IllegalTransition {
        /// Stage the fork is in.
        from: &'static str,
        /// Stage the requested transition leads to.
        to: &'static str,
    },
}

impl fmt::Display for HandoffError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleRetainedTarget => {
                formatter.write_str("retained destination was not revalidated")
            }
            Self::UnavailableTarget(mode) => {
                write!(formatter, "destination is unavailable: {}", mode.label())
            }
            Self::IllegalTransition { .. } => formatter.write_str("handoff transition is illegal"),
        }
    }
}

impl std::error::Error for HandoffError {}

/// The result of reconciling a lost handoff reply.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HandoffReconciliation {
    /// The child was already admitted; nothing new may be created.
    Adopted,
    /// No child exists yet; the prefix must be re-verified before a child is created.
    ReplayRequired,
}

/// Reports the next handoff stage after `from`, refusing any non-forward transition.
///
/// # Errors
///
/// Returns [`HandoffError::IllegalTransition`] when `to` is not the single forward successor of
/// `from`, including every transition out of the terminal `ChildRunning` stage.
pub fn next_handoff_stage(
    from: HandoffStage,
    to: HandoffStage,
) -> Result<HandoffStage, HandoffError> {
    let forward = match from {
        HandoffStage::PrefixSelected => matches!(to, HandoffStage::PrefixReplayed),
        HandoffStage::PrefixReplayed => matches!(to, HandoffStage::HandoffIntended),
        HandoffStage::HandoffIntended => matches!(to, HandoffStage::ChildAdmitted),
        HandoffStage::ChildAdmitted => matches!(to, HandoffStage::ChildRunning),
        HandoffStage::ChildRunning => false,
    };
    if forward {
        Ok(to)
    } else {
        Err(HandoffError::IllegalTransition {
            from: from.label(),
            to: to.label(),
        })
    }
}

/// Admits a handoff to a destination in `mode`, requiring revalidation for a retained target.
///
/// # Errors
///
/// Returns [`HandoffError::StaleRetainedTarget`] when a retained destination is not revalidated,
/// and [`HandoffError::UnavailableTarget`] for a destroyed, lost or changed destination.
pub fn admit_handoff(mode: HandoffMode, revalidated: bool) -> Result<HandoffMode, HandoffError> {
    match mode {
        HandoffMode::Retained if revalidated => Ok(mode),
        HandoffMode::Retained => Err(HandoffError::StaleRetainedTarget),
        other => Err(HandoffError::UnavailableTarget(other)),
    }
}

/// Reconciles a lost handoff reply without creating a duplicate child or skipping verification.
#[must_use]
pub fn reconcile_lost_handoff(child_admitted: bool) -> HandoffReconciliation {
    if child_admitted {
        HandoffReconciliation::Adopted
    } else {
        HandoffReconciliation::ReplayRequired
    }
}
