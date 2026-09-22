// SPDX-License-Identifier: MIT

//! A versioned, durable image of the prepared-dispatch receipt ledger.
//!
//! The receipt ledger is the only durable state of a held prepared dispatch: a restarted controller
//! is rebuilt from it, so the same approval can never be dispatched twice. On its own the ledger
//! lives in one process, so a composition that restarts re-derives an empty ledger and can write an
//! already-accepted application boundary a second time.
//!
//! [`DurableDispatchLedger`] is a faithful, versioned copy of the ledger's own state: every receipt
//! exactly as recorded and every terminal cancellation exactly as marked. It is deliberately not a
//! recomputed summary, so a reload can prove that the receipts it restores are the receipts that
//! were committed. The schema identifier is checked on load, so a reader never silently accepts a
//! newer image, and validation is fail-closed: an unknown version, a duplicate or mis-ordered
//! receipt, a malformed digest, or an identity that is both recorded and cancelled is refused
//! rather than repaired.
//!
//! [`DispatchLedgerPort`] is the owner-supplied reconciliation port that carries one image across a
//! restart. Nothing here reads or writes a file, a database or a network: the composition that owns
//! the store decides which sink a durable image is written to, and when.

use serde::{Deserialize, Serialize};

use super::dispatch_fences::valid_digest;
use super::dispatch_lifecycle::{DispatchLedger, DispatchOutcome, DispatchReceipt};
use super::valid_identity;
use std::fmt;

/// The schema identifier of the durable prepared-dispatch ledger image.
pub const DURABLE_DISPATCH_LEDGER_SCHEMA: &str = "ascension.dispatch-ledger.v1";

/// Why a durable prepared-dispatch ledger could not be written, loaded or trusted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DispatchLedgerError {
    /// The durable store could not be reached, so nothing about the recorded receipts is known.
    Unavailable,
    /// The durable image is mis-versioned, malformed or internally inconsistent.
    Invalid,
}

impl fmt::Display for DispatchLedgerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unavailable => "the durable dispatch ledger store is unavailable",
            Self::Invalid => "the durable dispatch ledger image is invalid",
        })
    }
}

impl std::error::Error for DispatchLedgerError {}

/// The durable image of one prepared-dispatch receipt ledger.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DurableDispatchLedger {
    /// Schema identifier. A reader refuses every other value.
    pub schema: String,
    /// Every receipt exactly as recorded, ordered by dispatch identity.
    pub receipts: Vec<DispatchReceipt>,
    /// Every terminal cancellation exactly as marked, ordered by dispatch identity.
    pub cancelled: Vec<String>,
}

impl DurableDispatchLedger {
    /// The durable image of one ledger.
    ///
    /// Receipts and cancellations keep the ledger's own order, so an image re-encodes to the bytes
    /// it was decoded from and a reloaded image is byte-identical to the committed one.
    #[must_use]
    pub fn capture(ledger: &DispatchLedger) -> Self {
        Self {
            schema: DURABLE_DISPATCH_LEDGER_SCHEMA.to_owned(),
            receipts: ledger.receipts().cloned().collect(),
            cancelled: ledger.cancelled().map(ToOwned::to_owned).collect(),
        }
    }

    /// Confirms the image is internally consistent before it is written or trusted.
    pub fn validate(&self) -> Result<(), DispatchLedgerError> {
        if self.schema != DURABLE_DISPATCH_LEDGER_SCHEMA {
            return Err(DispatchLedgerError::Invalid);
        }
        let mut previous: Option<&str> = None;
        for receipt in &self.receipts {
            validate_receipt(receipt)?;
            if previous.is_some_and(|bound| bound >= receipt.dispatch_id.as_str()) {
                return Err(DispatchLedgerError::Invalid);
            }
            previous = Some(receipt.dispatch_id.as_str());
        }
        // A receipt is authoritative for its identity, so an identity may not be both recorded and
        // cancelled, and neither list may carry an out-of-order or duplicated entry.
        let mut previous: Option<&str> = None;
        for dispatch_id in &self.cancelled {
            if !valid_identity(dispatch_id)
                || previous.is_some_and(|bound| bound >= dispatch_id.as_str())
                || self
                    .receipts
                    .iter()
                    .any(|receipt| receipt.dispatch_id == *dispatch_id)
            {
                return Err(DispatchLedgerError::Invalid);
            }
            previous = Some(dispatch_id.as_str());
        }
        Ok(())
    }

    /// Rebuilds a ledger from this image.
    ///
    /// Every receipt is re-validated and re-recorded under its own identity, so the reloaded ledger
    /// refuses a second dispatch of exactly the identities the committed one did.
    pub fn restore(&self) -> Result<DispatchLedger, DispatchLedgerError> {
        self.validate()?;
        let mut ledger = DispatchLedger::new();
        for receipt in &self.receipts {
            ledger.record_receipt(receipt.clone());
        }
        for dispatch_id in &self.cancelled {
            ledger.mark_cancelled(dispatch_id);
        }
        Ok(ledger)
    }
}

/// Confirms one receipt is a well-formed reconciliation record of one write attempt.
fn validate_receipt(receipt: &DispatchReceipt) -> Result<(), DispatchLedgerError> {
    let identity_ok = valid_identity(&receipt.dispatch_id)
        && valid_identity(&receipt.execution_id)
        && valid_identity(&receipt.adapter_id)
        && receipt.attempt_id.as_deref().is_none_or(valid_identity);
    let digest_ok =
        valid_digest(&receipt.approved_material_sha256) && valid_digest(&receipt.manifest_sha256);
    // One receipt accounts for exactly the single write attempt `resume` made, and an indeterminate
    // outcome wrote no application bytes.
    let counters_ok = receipt.boundary_writes == 1
        && receipt.gameplay_effects <= 1
        && (receipt.outcome == DispatchOutcome::WriteCompleted || receipt.written_bytes == 0);
    if !identity_ok || !digest_ok || !counters_ok {
        return Err(DispatchLedgerError::Invalid);
    }
    Ok(())
}

/// A durable, decision-level reconciliation port for the receipt ledger.
pub trait DispatchLedgerPort: fmt::Debug + Send {
    /// The image this composition last persisted, or `None` when it never persisted one.
    fn load(&mut self) -> Result<Option<DurableDispatchLedger>, DispatchLedgerError>;

    /// Persists one image, so a restarted composition reloads the same receipts.
    fn save(&mut self, ledger: &DurableDispatchLedger) -> Result<(), DispatchLedgerError>;
}

/// An inert port: nothing is persisted and no receipt is ever reloaded.
///
/// This is the served default, so a composition that attaches no durable port keeps the in-session
/// ledger rather than acquiring an undeclared durable surface.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopDispatchLedgerPort;

impl DispatchLedgerPort for NoopDispatchLedgerPort {
    fn load(&mut self) -> Result<Option<DurableDispatchLedger>, DispatchLedgerError> {
        Ok(None)
    }

    fn save(&mut self, _ledger: &DurableDispatchLedger) -> Result<(), DispatchLedgerError> {
        Ok(())
    }
}

/// A process-lifetime port: the image lives in memory for as long as this port is attached.
///
/// It is a real port rather than a simulation of one: a composition that rebuilds its served
/// session around one attached port reloads the receipts the previous session recorded. It is
/// deliberately not a durable store, because which store a served composition attaches is an owner
/// decision and not a default this crate may choose.
#[derive(Clone, Debug, Default)]
pub struct InMemoryDispatchLedgerPort {
    image: Option<DurableDispatchLedger>,
}

impl InMemoryDispatchLedgerPort {
    /// The image this port currently holds.
    #[must_use]
    pub fn image(&self) -> Option<&DurableDispatchLedger> {
        self.image.as_ref()
    }
}

impl DispatchLedgerPort for InMemoryDispatchLedgerPort {
    fn load(&mut self) -> Result<Option<DurableDispatchLedger>, DispatchLedgerError> {
        Ok(self.image.clone())
    }

    fn save(&mut self, ledger: &DurableDispatchLedger) -> Result<(), DispatchLedgerError> {
        ledger.validate()?;
        self.image = Some(ledger.clone());
        Ok(())
    }
}

impl DispatchLedger {
    /// The durable image of this ledger.
    #[must_use]
    pub fn durable(&self) -> DurableDispatchLedger {
        DurableDispatchLedger::capture(self)
    }
}

#[cfg(test)]
#[path = "dispatch_ledger_durable_tests.rs"]
mod tests;
