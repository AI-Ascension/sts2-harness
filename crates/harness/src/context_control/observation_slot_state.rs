// SPDX-License-Identifier: MIT

//! The identity-bound latest-only slot state machine (issue #112).
//!
//! This module owns the mutable slot itself: it keeps at most one effective value, retains
//! superseded and pinned values as attributed history, records a deterministic supersession
//! lineage, advances the approval fence on every replacement, and refuses to serve the
//! last-known value once a required refresh has failed. The published types and their admission
//! validation live in [`super::observation_slot`].

use super::observation_slot::{
    AdmittedObservationSlot, MAX_SLOT_HISTORY, MAX_SLOT_LINEAGE, OBSERVATION_SLOT_SCHEMA,
    ObservationSlotAdmission, ObservationSlotKey, ObservationSlotRefusal, SlotAdmission,
    SupersessionReason, SupersessionRecord, boundary_complete, slot_digest, valid_expiry,
    valid_timestamp,
};
use super::types::valid_id;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// One identity-bound latest-only slot.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationSelectionSlot {
    pub schema: String,
    pub key: ObservationSlotKey,
    pub slot_id: String,
    effective: Option<AdmittedObservationSlot>,
    history: Vec<AdmittedObservationSlot>,
    lineage: Vec<SupersessionRecord>,
    approval_fence: u64,
    refresh_failed: bool,
}

impl ObservationSelectionSlot {
    pub fn new(key: ObservationSlotKey) -> Result<Self, ObservationSlotRefusal> {
        if !key.valid() {
            return Err(ObservationSlotRefusal::InvalidIdentity);
        }
        let slot_id = format!("slot-{}", slot_digest(&key));
        Ok(Self {
            schema: OBSERVATION_SLOT_SCHEMA.to_owned(),
            key,
            slot_id,
            effective: None,
            history: Vec::new(),
            lineage: Vec::new(),
            approval_fence: 0,
            refresh_failed: false,
        })
    }

    #[must_use]
    pub fn key(&self) -> &ObservationSlotKey {
        &self.key
    }

    #[must_use]
    pub fn slot_id(&self) -> &str {
        &self.slot_id
    }

    /// The single effective value, if any. This is a raw accessor; callers that intend to act
    /// on the value must use [`Self::read`] so a failed refresh is not skipped.
    #[must_use]
    pub fn effective(&self) -> Option<&AdmittedObservationSlot> {
        self.effective.as_ref()
    }

    /// The effective value, or an explicit refusal. A slot whose required refresh failed
    /// refuses rather than returning the last-known value as current.
    pub fn read(&self, now: &str) -> Result<&AdmittedObservationSlot, ObservationSlotRefusal> {
        if self.refresh_failed {
            return Err(ObservationSlotRefusal::RefreshRequired);
        }
        let effective = self
            .effective
            .as_ref()
            .ok_or(ObservationSlotRefusal::PartialStateCatalog)?;
        if effective.expires_at.as_str() <= now {
            return Err(ObservationSlotRefusal::Expired);
        }
        Ok(effective)
    }

    /// Superseded and pinned values, newest first, each retaining its own attribution.
    #[must_use]
    pub fn history(&self) -> &[AdmittedObservationSlot] {
        &self.history
    }

    /// Deterministic supersession lineage, oldest first.
    #[must_use]
    pub fn lineage(&self) -> &[SupersessionRecord] {
        &self.lineage
    }

    #[must_use]
    pub fn approval_fence(&self) -> u64 {
        self.approval_fence
    }

    /// Whether an approval issued under `fence` is still current.
    #[must_use]
    pub fn approval_current(&self, fence: u64) -> bool {
        fence == self.approval_fence && self.approval_fence > 0
    }

    /// Record that a required refresh failed. The slot then refuses reads until a newer value
    /// is admitted.
    pub fn record_refresh_failure(&mut self) {
        self.refresh_failed = true;
    }

    #[must_use]
    pub fn refresh_failed(&self) -> bool {
        self.refresh_failed
    }

    /// Admit a candidate, replacing the effective value only when it is strictly newer.
    pub fn admit(
        &mut self,
        admission: &ObservationSlotAdmission,
        now: &str,
    ) -> Result<SlotAdmission, ObservationSlotRefusal> {
        if !valid_expiry(&admission.admitted_at, &admission.expires_at) || !valid_timestamp(now) {
            return Err(ObservationSlotRefusal::InvalidIdentity);
        }
        let candidate = admission.validate(&self.slot_id)?;
        if candidate.expires_at.as_str() <= now {
            return Err(ObservationSlotRefusal::Expired);
        }
        let Some(effective) = self.effective.as_ref() else {
            if self.lineage.len() >= MAX_SLOT_LINEAGE {
                return Err(ObservationSlotRefusal::Capacity);
            }
            self.approval_fence = self.approval_fence.saturating_add(1);
            self.lineage.push(SupersessionRecord {
                reason: SupersessionReason::Initial,
                superseded_state_id: None,
                superseded_observation_sha256: None,
                superseded_epoch: None,
                superseded_gate: None,
                admitted_state_id: candidate.boundary.state_id.clone(),
                admitted_observation_sha256: candidate.boundary.observation_sha256.clone(),
                admitted_epoch: candidate.boundary.controller_epoch,
                admitted_gate: candidate.boundary.gate_epoch,
                approval_fence: self.approval_fence,
            });
            self.effective = Some(candidate);
            self.refresh_failed = false;
            return Ok(SlotAdmission::Inserted {
                approval_fence: self.approval_fence,
            });
        };
        if same_value(effective, &candidate) {
            return Ok(SlotAdmission::Unchanged);
        }
        if candidate.ordering() < effective.ordering() {
            return Err(ObservationSlotRefusal::OutOfOrder);
        }
        if candidate.ordering() == effective.ordering() {
            return Err(ObservationSlotRefusal::ConflictingSameSequence);
        }
        if self.history.len() >= MAX_SLOT_HISTORY || self.lineage.len() >= MAX_SLOT_LINEAGE {
            return Err(ObservationSlotRefusal::Capacity);
        }
        let previous = effective.clone();
        let mut records = std::mem::take(&mut self.history);
        if previous.pinned {
            records.insert(0, previous.clone());
        }
        records.truncate(MAX_SLOT_HISTORY);
        self.history = records;
        self.approval_fence = self.approval_fence.saturating_add(1);
        self.lineage.push(SupersessionRecord {
            reason: SupersessionReason::NewerGeneration,
            superseded_state_id: Some(previous.boundary.state_id.clone()),
            superseded_observation_sha256: Some(previous.boundary.observation_sha256.clone()),
            superseded_epoch: Some(previous.boundary.controller_epoch),
            superseded_gate: Some(previous.boundary.gate_epoch),
            admitted_state_id: candidate.boundary.state_id.clone(),
            admitted_observation_sha256: candidate.boundary.observation_sha256.clone(),
            admitted_epoch: candidate.boundary.controller_epoch,
            admitted_gate: candidate.boundary.gate_epoch,
            approval_fence: self.approval_fence,
        });
        self.effective = Some(candidate);
        self.refresh_failed = false;
        Ok(SlotAdmission::Superseded {
            approval_fence: self.approval_fence,
        })
    }
}

fn same_value(left: &AdmittedObservationSlot, right: &AdmittedObservationSlot) -> bool {
    left.boundary.external_eq(&right.boundary)
}

/// Reject a slot image whose retained values are not internally consistent.
pub fn validate_slot_image(slot: &ObservationSelectionSlot) -> Result<(), ObservationSlotRefusal> {
    if slot.schema != OBSERVATION_SLOT_SCHEMA
        || !slot.key.valid()
        || !valid_id(&slot.slot_id)
        || slot.history.len() > MAX_SLOT_HISTORY
        || slot.lineage.len() > MAX_SLOT_LINEAGE
    {
        return Err(ObservationSlotRefusal::InvalidIdentity);
    }
    let mut effective_state = BTreeSet::new();
    if let Some(effective) = &slot.effective
        && (effective.schema != OBSERVATION_SLOT_SCHEMA
            || effective.slot_id != slot.slot_id
            || effective.key != slot.key
            || !boundary_complete(&effective.boundary)
            || !valid_expiry(&effective.admitted_at, &effective.expires_at)
            || !effective_state.insert(effective.boundary.state_id.clone()))
    {
        return Err(ObservationSlotRefusal::InvalidIdentity);
    }
    for entry in &slot.history {
        if entry.schema != OBSERVATION_SLOT_SCHEMA
            || entry.slot_id != slot.slot_id
            || entry.key != slot.key
            || !boundary_complete(&entry.boundary)
            || !valid_expiry(&entry.admitted_at, &entry.expires_at)
        {
            return Err(ObservationSlotRefusal::InvalidIdentity);
        }
    }
    if slot.approval_fence > 0 && slot.effective.is_none() {
        return Err(ObservationSlotRefusal::InvalidIdentity);
    }
    Ok(())
}
