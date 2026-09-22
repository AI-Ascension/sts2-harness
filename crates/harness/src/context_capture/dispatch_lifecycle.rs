// SPDX-License-Identifier: MIT

//! Durable lifecycle vocabulary for one held prepared dispatch.
//!
//! A held dispatch is drafted, acknowledged and then dispatched exactly once.  The receipt ledger
//! and its terminal cancellations are the only durable state: a restarted controller is rebuilt
//! from them, so the same approval can never be dispatched twice.  The ledger's wire vocabulary
//! lives in [`super::dispatch_ledger_durable`], which carries that state across a process restart
//! through an owner-supplied reconciliation port.

use super::CaptureBoundary;
use super::dispatch_error::DispatchError;
use super::dispatch_fences::DispatchFences;
use super::dispatch_material::{BoundaryManifestEntry, PreparedApplicationInput};
use super::dispatch_support::EffectiveContextClaim;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Lifecycle of one held dispatch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DispatchState {
    /// Bound and previewable.  Nothing is written and nothing is approved for a write.
    Drafted,
    /// Acknowledged by the owner and held.  Still nothing written.
    CommittedHeld,
    /// The approved material left the approval: the write completed, or its outcome is
    /// indeterminate.  Either way no further write may be sent.
    Dispatched,
    /// Drift, revocation or a stop request fenced the approval before any write.
    Stale,
    /// Cancelled before any write.  Cancellation is terminal.
    Cancelled,
}

impl DispatchState {
    /// Stable wire label for the state.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Drafted => "drafted",
            Self::CommittedHeld => "committed_held",
            Self::Dispatched => "dispatched",
            Self::Stale => "stale",
            Self::Cancelled => "cancelled",
        }
    }
}

/// The outcome of the single write attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum DispatchOutcome {
    /// The port recorded the approved material.
    WriteCompleted,
    /// The transport outcome is indeterminate.  A possible provider write forbids a resend.
    Unknown,
}

impl DispatchOutcome {
    /// Stable wire label for the outcome.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WriteCompleted => "write_completed",
            Self::Unknown => "unknown",
        }
    }
}

impl From<DispatchOutcome> for String {
    fn from(outcome: DispatchOutcome) -> Self {
        outcome.as_str().to_owned()
    }
}

impl TryFrom<String> for DispatchOutcome {
    type Error = DispatchError;

    /// Parses the stable wire label. An unknown label is refused rather than approximated.
    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "write_completed" => Ok(Self::WriteCompleted),
            "unknown" => Ok(Self::Unknown),
            _ => Err(DispatchError::InvalidMaterial),
        }
    }
}

/// Durable reconciliation record for exactly one dispatch attempt.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DispatchReceipt {
    /// Identity of the held dispatch this receipt reconciles.
    pub dispatch_id: String,
    /// Execution identity that owns the approved material.
    pub execution_id: String,
    /// Attempt identity within the execution, when one was reserved.
    pub attempt_id: Option<String>,
    /// Adapter id the material was prepared for.
    pub adapter_id: String,
    /// Exact application boundary the port wrote.
    pub boundary: CaptureBoundary,
    /// Digest of the ordered component bytes.
    pub approved_material_sha256: String,
    /// Digest of the ordered manifest.
    pub manifest_sha256: String,
    /// Outcome of the single write attempt.
    pub outcome: DispatchOutcome,
    /// Application bytes written, or zero when the outcome is indeterminate.
    pub written_bytes: usize,
    /// Application-boundary writes this receipt accounts for.
    pub boundary_writes: u32,
    /// Gameplay effects this receipt accounts for.
    pub gameplay_effects: u32,
}

/// Append-only receipts plus terminal cancellations.
///
/// A restarted controller is rebuilt from this ledger, so a resend cannot be mistaken for a fresh
/// dispatch.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DispatchLedger {
    receipts: BTreeMap<String, DispatchReceipt>,
    cancelled: BTreeSet<String>,
}

impl DispatchLedger {
    /// An empty ledger.
    pub fn new() -> Self {
        Self::default()
    }

    /// The recorded receipt for one dispatch, when a write was already attempted.
    pub fn receipt(&self, dispatch_id: &str) -> Option<&DispatchReceipt> {
        self.receipts.get(dispatch_id)
    }

    /// Whether the dispatch was cancelled terminally.
    pub fn is_cancelled(&self, dispatch_id: &str) -> bool {
        self.cancelled.contains(dispatch_id)
    }

    /// Number of recorded receipts.
    pub fn receipt_count(&self) -> usize {
        self.receipts.len()
    }

    /// Every retained receipt, ordered by dispatch identity.
    pub(crate) fn receipts(&self) -> impl Iterator<Item = &DispatchReceipt> {
        self.receipts.values()
    }

    /// Every terminal cancellation, ordered by dispatch identity.
    pub(crate) fn cancelled(&self) -> impl Iterator<Item = &str> {
        self.cancelled.iter().map(String::as_str)
    }

    /// Records one receipt.  A recorded receipt is authoritative for its dispatch identity.
    pub(crate) fn record_receipt(&mut self, receipt: DispatchReceipt) {
        self.receipts.insert(receipt.dispatch_id.clone(), receipt);
    }

    /// Marks one dispatch identity as cancelled terminally.
    pub(crate) fn mark_cancelled(&mut self, dispatch_id: &str) {
        self.cancelled.insert(dispatch_id.to_owned());
    }
}

/// One held dispatch together with the fences it was approved against.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedDispatch {
    /// Identity of this dispatch.
    pub dispatch_id: String,
    /// Exact approved material.
    pub input: PreparedApplicationInput,
    /// Fences the approval was bound to.
    pub fences: DispatchFences,
    /// Lifecycle state.
    pub state: DispatchState,
    approved_stop_epoch: u64,
}

impl PreparedDispatch {
    /// Binds one prepared input as a drafted dispatch under the current stop epoch.
    pub(crate) fn new(
        dispatch_id: &str,
        input: PreparedApplicationInput,
        fences: DispatchFences,
        approved_stop_epoch: u64,
    ) -> Self {
        Self {
            dispatch_id: dispatch_id.to_owned(),
            input,
            fences,
            state: DispatchState::Drafted,
            approved_stop_epoch,
        }
    }

    /// Stop epoch that was current when this dispatch was bound.
    pub const fn approved_stop_epoch(&self) -> u64 {
        self.approved_stop_epoch
    }
}

/// Bounded, byte-free preview of one held dispatch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DispatchPreview {
    /// Identity of this dispatch.
    pub dispatch_id: String,
    /// Adapter id the material was prepared for.
    pub adapter_id: String,
    /// Exact application boundary the adapter writes.
    pub boundary: CaptureBoundary,
    /// Exactness a consumer may publish.
    pub claim: EffectiveContextClaim,
    /// Lifecycle state at preview time.
    pub state: DispatchState,
    /// Digest of the ordered component bytes.
    pub approved_material_sha256: String,
    /// Digest of the ordered manifest.
    pub manifest_sha256: String,
    /// Ordered manifest entries.
    pub entries: Vec<BoundaryManifestEntry>,
    /// Always `false`: a preview publishes bounded metadata, never the approved bytes.
    pub bytes_exposed: bool,
}

/// Bounded metadata read of one held dispatch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DispatchMetadata {
    /// Identity of this dispatch.
    pub dispatch_id: String,
    /// Execution identity that owns the approved material.
    pub execution_id: String,
    /// Attempt identity within the execution, when one was reserved.
    pub attempt_id: Option<String>,
    /// Adapter id the material was prepared for.
    pub adapter_id: String,
    /// Exact application boundary the adapter writes.
    pub boundary: CaptureBoundary,
    /// Exactness a consumer may publish.
    pub claim: EffectiveContextClaim,
    /// Lifecycle state at read time.
    pub state: DispatchState,
    /// Number of exact components.
    pub component_count: usize,
    /// Total approved application bytes.
    pub approved_bytes: usize,
    /// Digest of the ordered component bytes.
    pub approved_material_sha256: String,
    /// Digest of the ordered manifest.
    pub manifest_sha256: String,
    /// Retained receipt, when a write was already attempted.
    pub receipt: Option<DispatchReceipt>,
}
