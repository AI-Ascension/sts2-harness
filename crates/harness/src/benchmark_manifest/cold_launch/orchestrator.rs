// SPDX-License-Identifier: MIT

//! Orchestrator state shared across trials: destination leases and live process births.
//!
//! A process birth token is live for exactly one trial, so a second trial cannot adopt it. A
//! quarantined destination is never leased again.

use std::collections::BTreeMap;

use super::error::ColdLaunchError;
use super::lease::{Destination, LeaseAllocator};
use super::process::ProcessBirth;

/// Shared allocation and live-process registry for cold-launch trials.
pub struct ColdLaunchOrchestrator {
    allocator: LeaseAllocator,
    live_births: BTreeMap<String, String>,
    quarantined: BTreeMap<String, String>,
}

impl ColdLaunchOrchestrator {
    /// Builds an orchestrator with a concurrent-allocation bound.
    ///
    /// # Errors
    ///
    /// Returns [`ColdLaunchError::InvalidAllocationBound`] outside the supported range.
    pub fn new(bound: usize) -> Result<Self, ColdLaunchError> {
        Ok(Self {
            allocator: LeaseAllocator::new(bound)?,
            live_births: BTreeMap::new(),
            quarantined: BTreeMap::new(),
        })
    }

    /// Leases a fresh destination to one trial, refusing a quarantined or foreign destination.
    ///
    /// # Errors
    ///
    /// Returns [`ColdLaunchError::DestinationQuarantined`], [`ColdLaunchError::DestinationLeased`]
    /// or [`ColdLaunchError::AllocationBoundReached`].
    pub fn lease_destination(
        &mut self,
        destination_id: &str,
        trial_key: &str,
    ) -> Result<Destination, ColdLaunchError> {
        if let Some(holder) = self.quarantined.get(destination_id) {
            return Err(ColdLaunchError::DestinationQuarantined(holder.clone()));
        }
        self.allocator.lease(destination_id, trial_key)
    }

    /// Attests a live process birth for one trial; a token is live for exactly one trial.
    ///
    /// # Errors
    ///
    /// Returns [`ColdLaunchError::ReusedLiveProcess`] when the token is already attested for a
    /// different trial and the process rejection for a malformed birth.
    pub fn attest_birth(
        &mut self,
        trial_key: &str,
        birth: &ProcessBirth,
    ) -> Result<(), ColdLaunchError> {
        birth.validate()?;
        if let Some(holder) = self.live_births.get(&birth.birth_token)
            && holder != trial_key
        {
            return Err(ColdLaunchError::ReusedLiveProcess {
                birth_token: birth.birth_token.clone(),
            });
        }
        self.live_births
            .insert(birth.birth_token.clone(), trial_key.to_owned());
        Ok(())
    }

    /// Retires a birth token, returning the trial that held it.
    pub fn retire_birth(&mut self, birth_token: &str) -> Option<String> {
        self.live_births.remove(birth_token)
    }

    /// Releases a destination lease, returning the trial that held it.
    pub fn release_destination(&mut self, destination_id: &str) -> Option<String> {
        self.allocator.release(destination_id)
    }

    /// Quarantines a destination so it is never leased again.
    pub fn quarantine_destination(&mut self, destination_id: &str, trial_key: &str) {
        self.allocator.release(destination_id);
        self.quarantined
            .insert(destination_id.to_owned(), trial_key.to_owned());
    }

    /// Number of live process births.
    #[must_use]
    pub fn live_births(&self) -> usize {
        self.live_births.len()
    }

    /// Number of active destination leases.
    #[must_use]
    pub fn active_leases(&self) -> usize {
        self.allocator.active()
    }
}
