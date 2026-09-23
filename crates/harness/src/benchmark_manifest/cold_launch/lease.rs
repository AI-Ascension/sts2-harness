// SPDX-License-Identifier: MIT

//! Bounded exclusive leases over writable trial destinations.
//!
//! A destination is provisioned from the immutable baseline for exactly one trial. The allocator
//! never hands the same destination to two trials, and a released destination is available again
//! only after it was cleaned or deliberately returned.

use std::collections::BTreeMap;

use super::error::ColdLaunchError;

/// Maximum concurrent destination allocations for one cold-launch orchestrator.
pub const MAX_CONCURRENT_ALLOCATIONS: usize = 32;

/// A writable destination provisioned from the baseline for exactly one trial.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Destination {
    /// Gateway-owned destination identifier.
    pub destination_id: String,
    /// Trial that holds the exclusive lease.
    pub trial_key: String,
}

/// Bounded allocator of exclusive destinations; never hands one out twice.
pub struct LeaseAllocator {
    bound: usize,
    leases: BTreeMap<String, String>,
}

impl LeaseAllocator {
    /// Builds an allocator with a bound in `1..=MAX_CONCURRENT_ALLOCATIONS`.
    ///
    /// # Errors
    ///
    /// Returns [`ColdLaunchError::InvalidAllocationBound`] outside that range.
    pub fn new(bound: usize) -> Result<Self, ColdLaunchError> {
        if bound == 0 || bound > MAX_CONCURRENT_ALLOCATIONS {
            return Err(ColdLaunchError::InvalidAllocationBound);
        }
        Ok(Self {
            bound,
            leases: BTreeMap::new(),
        })
    }

    /// Leases a destination to one trial; a repeat lease to the same trial is idempotent.
    ///
    /// # Errors
    ///
    /// Returns [`ColdLaunchError::DestinationLeased`] when another trial holds it and
    /// [`ColdLaunchError::AllocationBoundReached`] when the bound is full.
    pub fn lease(
        &mut self,
        destination_id: &str,
        trial_key: &str,
    ) -> Result<Destination, ColdLaunchError> {
        if let Some(holder) = self.leases.get(destination_id)
            && holder != trial_key
        {
            return Err(ColdLaunchError::DestinationLeased {
                destination_id: destination_id.to_owned(),
                trial_key: holder.clone(),
            });
        }
        if !self.leases.contains_key(destination_id) {
            if self.leases.len() >= self.bound {
                return Err(ColdLaunchError::AllocationBoundReached);
            }
            self.leases
                .insert(destination_id.to_owned(), trial_key.to_owned());
        }
        Ok(Destination {
            destination_id: destination_id.to_owned(),
            trial_key: trial_key.to_owned(),
        })
    }

    /// Releases a destination, returning the trial that held it.
    pub fn release(&mut self, destination_id: &str) -> Option<String> {
        self.leases.remove(destination_id)
    }

    /// Returns the trial holding a destination lease, if any.
    #[must_use]
    pub fn holder(&self, destination_id: &str) -> Option<&str> {
        self.leases.get(destination_id).map(String::as_str)
    }

    /// Number of active leases.
    #[must_use]
    pub fn active(&self) -> usize {
        self.leases.len()
    }
}
