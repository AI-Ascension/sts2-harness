// SPDX-License-Identifier: MIT

//! Registry and restart image for identity-bound observation slots (issue #112).
//!
//! The store owns one [`ObservationSelectionSlot`] per [`ObservationSlotKey`]. Its restart
//! image is a pure capture of the slots, so restoring it rebuilds *the* effective selection
//! for each identity rather than a second, competing one. Because the image also carries each
//! slot's approval fence, an approval issued before a restart against an older fence is
//! refused after the restart; the slot never silently revalidates a stale approval.

use super::observation_slot::{
    AdmittedObservationSlot, ObservationSlotAdmission, ObservationSlotKey, ObservationSlotRefusal,
    SlotAdmission,
};
use super::observation_slot_state::{ObservationSelectionSlot, validate_slot_image};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Stable schema identity of a captured observation-slot image.
pub const OBSERVATION_SLOT_IMAGE_SCHEMA: &str =
    "ascension.context-control.observation-slot-image.v1";

/// Upper bound on distinct slots one store may hold.
pub const MAX_OBSERVATION_SLOTS: usize = 64;

/// One store of identity-bound latest-only slots.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationSlotStore {
    slots: BTreeMap<ObservationSlotKey, ObservationSelectionSlot>,
}

impl ObservationSlotStore {
    #[must_use]
    pub fn new() -> Self {
        Self {
            slots: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn slot(&self, key: &ObservationSlotKey) -> Option<&ObservationSelectionSlot> {
        self.slots.get(key)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Every slot's single effective value, in deterministic slot-key order.
    #[must_use]
    pub fn effective_selections(&self) -> Vec<&AdmittedObservationSlot> {
        self.slots
            .values()
            .filter_map(ObservationSelectionSlot::effective)
            .collect()
    }

    /// Admit a candidate into the slot for its own key, creating that slot on first use.
    pub fn admit(
        &mut self,
        admission: &ObservationSlotAdmission,
        now: &str,
    ) -> Result<SlotAdmission, ObservationSlotRefusal> {
        if !self.slots.contains_key(&admission.key) {
            if self.slots.len() >= MAX_OBSERVATION_SLOTS {
                return Err(ObservationSlotRefusal::Capacity);
            }
            let slot = ObservationSelectionSlot::new(admission.key.clone())?;
            self.slots.insert(admission.key.clone(), slot);
        }
        self.slots
            .get_mut(&admission.key)
            .ok_or(ObservationSlotRefusal::InvalidIdentity)?
            .admit(admission, now)
    }

    /// Record a failed required refresh for one identity. The slot then refuses reads until a
    /// strictly newer value is admitted.
    pub fn record_refresh_failure(
        &mut self,
        key: &ObservationSlotKey,
    ) -> Result<(), ObservationSlotRefusal> {
        self.slots
            .get_mut(key)
            .ok_or(ObservationSlotRefusal::InvalidIdentity)?
            .record_refresh_failure();
        Ok(())
    }

    /// Capture the store as a restartable image.
    pub fn capture(&self) -> Result<DurableObservationSlots, ObservationSlotRefusal> {
        let image = DurableObservationSlots {
            schema: OBSERVATION_SLOT_IMAGE_SCHEMA.to_owned(),
            slots: self.slots.values().cloned().collect(),
        };
        image.validate()?;
        Ok(image)
    }
}

impl Default for ObservationSlotStore {
    fn default() -> Self {
        Self::new()
    }
}

/// A captured image of every slot, used to restart a store.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DurableObservationSlots {
    pub schema: String,
    pub slots: Vec<ObservationSelectionSlot>,
}

impl DurableObservationSlots {
    /// Confirm the image is internally consistent before it is written or trusted.
    pub fn validate(&self) -> Result<(), ObservationSlotRefusal> {
        if self.schema != OBSERVATION_SLOT_IMAGE_SCHEMA
            || self.slots.len() > MAX_OBSERVATION_SLOTS
            || self.slots.is_empty()
        {
            return Err(ObservationSlotRefusal::InvalidIdentity);
        }
        let mut keys = std::collections::BTreeSet::new();
        for slot in &self.slots {
            validate_slot_image(slot)?;
            if !keys.insert(slot.key.clone()) {
                return Err(ObservationSlotRefusal::InvalidIdentity);
            }
        }
        Ok(())
    }

    /// Rebuild the store from this image.
    ///
    /// Every slot is revalidated, and each identity contributes exactly one effective value.
    /// The approval fences carried in the image are preserved, so an approval issued before
    /// the restart against an older fence remains invalid afterwards.
    pub fn restore(&self) -> Result<ObservationSlotStore, ObservationSlotRefusal> {
        self.validate()?;
        let mut slots = BTreeMap::new();
        for slot in &self.slots {
            if slots.insert(slot.key.clone(), slot.clone()).is_some() {
                return Err(ObservationSlotRefusal::InvalidIdentity);
            }
        }
        Ok(ObservationSlotStore { slots })
    }
}
