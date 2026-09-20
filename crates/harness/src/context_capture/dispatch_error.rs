// SPDX-License-Identifier: MIT

//! Typed refusals for prepared dispatch.

use super::CaptureError;
use super::dispatch_fences::DriftAxis;
use std::fmt;

/// Why a prepared dispatch was refused, fenced or left indeterminate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DispatchError {
    /// An identity, adapter or fence did not match the prepared binding.
    InvalidBinding,
    /// The prepared material is empty, unordered or malformed.
    InvalidMaterial,
    /// The prepared material exceeds its byte bound.
    TooLarge,
    /// The adapter has no exact application boundary.
    UnsupportedAdapter,
    /// No prepared dispatch is held under this identity.
    UnknownDispatch,
    /// A dispatch was already recorded or is already held under this identity.
    DuplicateDispatch,
    /// The dispatch must be committed and held before it may be resumed.
    NotHeld,
    /// Drift, revocation or a stop request fenced the approval before any write.
    Stale,
    /// The dispatch was cancelled; cancellation is terminal.
    Cancelled,
    /// One bound axis drifted away from the held approval.
    Drift(DriftAxis),
    /// The write port cannot record exact material, so no exactness may be claimed.
    CaptureDisabled,
    /// The port reported an indeterminate transport outcome, for example a lost reply.  The
    /// recorded receipt is authoritative, so the approved material is never resent.
    Indeterminate,
    /// The capture port refused to record an approved component.
    Capture(CaptureError),
}

impl fmt::Display for DispatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidBinding => "prepared dispatch binding is invalid",
            Self::InvalidMaterial => "prepared dispatch material is invalid",
            Self::TooLarge => "prepared dispatch material exceeds its bound",
            Self::UnsupportedAdapter => "adapter has no exact application boundary",
            Self::UnknownDispatch => "no prepared dispatch is held under this identity",
            Self::DuplicateDispatch => "prepared dispatch identity is already recorded",
            Self::NotHeld => "prepared dispatch is not committed and held",
            Self::Stale => "prepared dispatch approval is stale",
            Self::Cancelled => "prepared dispatch was cancelled",
            Self::Drift(axis) => {
                return write!(formatter, "prepared dispatch drifted: {}", axis.as_str());
            }
            Self::CaptureDisabled => "write port cannot record exact application material",
            Self::Indeterminate => "prepared dispatch transport outcome is indeterminate",
            Self::Capture(error) => {
                return write!(formatter, "capture refused prepared material: {error}");
            }
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for DispatchError {}

impl From<CaptureError> for DispatchError {
    fn from(error: CaptureError) -> Self {
        Self::Capture(error)
    }
}
