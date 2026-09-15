// SPDX-License-Identifier: MIT

use std::fmt;

use serde::Serialize;
use serde_json::{Value, json};

use super::contract::Occurrence;
use super::{Manifest, ManifestError, commitment, encode, validation};

/// The exact evidence available from this library, never complete native verification.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrialStatus {
    /// Immutable declarations and an original occurrence have been checked.
    Planned,
    /// A structurally consistent existing settled receipt matches that occurrence.
    SeedReceiptBound,
}

/// Immutable trial association. A failed binding leaves the original plan intact.
/// The name remains stable after binding so callers may safely recheck the same receipt.
#[derive(Clone)]
pub struct PlannedTrial {
    manifest: Manifest,
    occurrence: Occurrence,
    receipt_digest: Option<String>,
}

impl fmt::Debug for PlannedTrial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlannedTrial")
            .field("status", &self.status())
            .finish_non_exhaustive()
    }
}

impl PlannedTrial {
    pub(super) fn new(manifest: Manifest, occurrence: Occurrence) -> Self {
        Self {
            manifest,
            occurrence,
            receipt_digest: None,
        }
    }

    /// Returns the immutable declarations associated with this occurrence.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Reports only the evidence checked by this effect-free library.
    pub fn status(&self) -> TrialStatus {
        if self.receipt_digest.is_some() {
            TrialStatus::SeedReceiptBound
        } else {
            TrialStatus::Planned
        }
    }

    /// Checks a bounded legacy seeded receipt wrapper and returns a new bound record.
    /// Rebinding identical canonical bytes is idempotent; other receipt bytes conflict.
    /// This does not authenticate the sender, inspect raw MCP history, persist anything
    /// or certify fields absent from seeded-run-v1 (profile/platform/complete RNG).
    pub fn bind_seed_receipt(&self, bytes: &[u8]) -> Result<Self, ManifestError> {
        let receipt: Value = validation::parse(bytes)?;
        check_wrapper(&receipt)?;
        validation::schema_valid(&receipt["settled"], false)
            .map_err(|_| ManifestError::InvalidReceipt)?;
        if !crate::recorded_run::recorded_run_seed::valid(&receipt) {
            return Err(ManifestError::InvalidReceipt);
        }
        self.check_identity(&receipt)?;
        self.check_inputs(&receipt)?;
        let receipt_digest = commitment("seed-receipt", &encode(&receipt)?);
        if self
            .receipt_digest
            .as_ref()
            .is_some_and(|old| old != &receipt_digest)
        {
            return Err(ManifestError::ReceiptConflict);
        }
        Ok(Self {
            manifest: self.manifest.clone(),
            occurrence: self.occurrence.clone(),
            receipt_digest: Some(receipt_digest),
        })
    }

    fn check_identity(&self, receipt: &Value) -> Result<(), ManifestError> {
        let o = &self.occurrence;
        let v = &receipt["settled"];
        for (field, expected) in [
            ("instance_id", &o.instance_id),
            ("session_id", &o.gateway_session_id),
            ("lease_id", &o.lease_id),
            ("operation_id", &o.operation_id),
            ("run_mode", &o.run_mode),
        ] {
            if v[field].as_str() != Some(expected.as_str()) {
                return Err(ManifestError::ReceiptIdentityMismatch);
            }
        }
        if receipt["plan_digest"] != o.plan_digest
            || receipt["entry_ordinal"].as_u64() != Some(o.entry_ordinal)
            || receipt["operation_id"] != o.operation_id
            || v["lease_epoch"].as_u64() != Some(o.lease_epoch)
            || v["generation"].as_u64() != Some(o.request_generation)
        {
            return Err(ManifestError::ReceiptIdentityMismatch);
        }
        Ok(())
    }

    fn check_inputs(&self, receipt: &Value) -> Result<(), ManifestError> {
        let g = &self.manifest.document.gameplay;
        let v = &receipt["settled"];
        if receipt["requested_seed"] != g.requested_seed
            || v["requested_seed"] != g.requested_seed
            || v["canonical_seed"] != g.effective_seed
        {
            return Err(ManifestError::ReceiptSeedMismatch);
        }
        let context = serde_json::to_value(&g.selected_context)
            .map_err(|_| ManifestError::InvalidDocument)?;
        if v["selected_context"] != context
            || v["context_digest"] != g.selected_context.context_digest
        {
            return Err(ManifestError::ReceiptContextMismatch);
        }
        Ok(())
    }

    /// Exports private association metadata for an authorized store. A stored status
    /// alone is not accepted as evidence: restore the plan and recheck retained receipt bytes.
    pub fn export_private(&self) -> Result<Vec<u8>, ManifestError> {
        encode(&json!({
            "version": "ascension.benchmark-trial.v1",
            "artifact_digest": self.manifest.artifact_digest_private(),
            "configuration_digest": self.manifest.configuration_digest_private(),
            "experiment_digest": self.manifest.experiment_digest_private(),
            "occurrence": self.occurrence,
            "status": self.status(),
            "receipt_digest": self.receipt_digest,
        }))
    }
}

// Retained exchanges are opaque private archival data, not additional evidence.
// Close the wrapper vocabulary while leaving those legacy exchange shapes intact.
fn check_wrapper(receipt: &Value) -> Result<(), ManifestError> {
    let object = receipt.as_object().ok_or(ManifestError::InvalidReceipt)?;
    const REQUIRED: [&str; 7] = [
        "operation_id",
        "requested_seed",
        "plan_digest",
        "entry_ordinal",
        "settled",
        "start",
        "reconcile",
    ];
    if REQUIRED.iter().any(|key| !object.contains_key(*key))
        || object.keys().any(|key| {
            !REQUIRED.contains(&key.as_str())
                && !["start_error", "duplicate_start"].contains(&key.as_str())
        })
        || !(receipt["start"].is_null() || receipt["start"].is_object())
        || !receipt["reconcile"]
            .as_array()
            .is_some_and(|v| v.len() <= 64 && v.iter().all(Value::is_object))
        || object
            .get("duplicate_start")
            .is_some_and(|v| !v.is_object())
        || object
            .get("start_error")
            .is_some_and(|v| !v.as_str().is_some_and(|s| s.len() <= 1024))
    {
        return Err(ManifestError::InvalidReceipt);
    }
    Ok(())
}
