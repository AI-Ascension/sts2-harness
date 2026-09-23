// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used, dead_code)]

// AC4: concurrent attempts preserve lease isolation and completed evidence, and a bounded allocator
// never hands one destination to two trials.

#[path = "cold_launch/fixtures.rs"]
mod fixtures;

use fixtures::birth;
use sts2_harness::benchmark_manifest::cold_launch::{
    ColdLaunchError, ColdLaunchOrchestrator, LeaseAllocator, MAX_CONCURRENT_ALLOCATIONS,
};

fn leased(destination_id: &str, trial_key: &str) -> ColdLaunchError {
    ColdLaunchError::DestinationLeased {
        destination_id: destination_id.to_owned(),
        trial_key: trial_key.to_owned(),
    }
}

#[test]
fn concurrent_leases_are_exclusive_and_bounded() {
    let mut allocator = LeaseAllocator::new(2).unwrap();
    allocator.lease("dest-one", "trial-one").unwrap();
    allocator.lease("dest-two", "trial-two").unwrap();
    assert_eq!(allocator.active(), 2);
    assert_eq!(
        allocator.lease("dest-three", "trial-three"),
        Err(ColdLaunchError::AllocationBoundReached)
    );
    assert_eq!(
        allocator.lease("dest-one", "trial-two"),
        Err(leased("dest-one", "trial-one"))
    );
    allocator
        .lease("dest-one", "trial-one")
        .expect("a repeat lease to the same trial is idempotent");
    assert_eq!(allocator.active(), 2);
    assert_eq!(allocator.holder("dest-one"), Some("trial-one"));
    assert_eq!(allocator.holder("dest-absent"), None);
}

#[test]
fn a_released_destination_frees_its_slot_and_reports_its_holder() {
    let mut allocator = LeaseAllocator::new(2).unwrap();
    allocator.lease("dest-one", "trial-one").unwrap();
    allocator.lease("dest-two", "trial-two").unwrap();
    assert_eq!(allocator.release("dest-one"), Some("trial-one".to_owned()));
    assert_eq!(allocator.active(), 1);
    assert!(allocator.lease("dest-three", "trial-three").is_ok());
    assert_eq!(allocator.active(), 2);
    assert_eq!(allocator.release("dest-one"), None);
}

#[test]
fn a_quarantined_destination_is_never_leased_again() {
    let mut orchestrator = ColdLaunchOrchestrator::new(1).unwrap();
    orchestrator
        .lease_destination("dest-one", "trial-one")
        .unwrap();
    assert_eq!(orchestrator.active_leases(), 1);
    assert_eq!(
        orchestrator.lease_destination("dest-two", "trial-two"),
        Err(ColdLaunchError::AllocationBoundReached)
    );
    orchestrator.quarantine_destination("dest-one", "trial-one");
    assert_eq!(orchestrator.active_leases(), 0);
    assert_eq!(
        orchestrator.lease_destination("dest-one", "trial-two"),
        Err(ColdLaunchError::DestinationQuarantined(
            "trial-one".to_owned()
        ))
    );
    assert!(
        orchestrator
            .lease_destination("dest-two", "trial-two")
            .is_ok()
    );
}

#[test]
fn retiring_a_birth_and_releasing_a_destination_report_their_holder() {
    let mut orchestrator = ColdLaunchOrchestrator::new(2).unwrap();
    orchestrator
        .lease_destination("dest-one", "trial-one")
        .unwrap();
    orchestrator
        .attest_birth("trial-one", &birth("token-one", 1))
        .unwrap();
    assert_eq!(
        orchestrator.release_destination("dest-one"),
        Some("trial-one".to_owned())
    );
    assert_eq!(
        orchestrator.retire_birth("token-one"),
        Some("trial-one".to_owned())
    );
    assert_eq!(orchestrator.release_destination("dest-one"), None);
    assert_eq!(orchestrator.retire_birth("token-one"), None);
    assert_eq!(orchestrator.live_births(), 0);
    assert_eq!(orchestrator.active_leases(), 0);
}

#[test]
fn allocation_bounds_outside_the_supported_range_are_refused() {
    assert!(matches!(
        LeaseAllocator::new(0),
        Err(ColdLaunchError::InvalidAllocationBound)
    ));
    assert!(LeaseAllocator::new(MAX_CONCURRENT_ALLOCATIONS).is_ok());
    assert!(matches!(
        LeaseAllocator::new(MAX_CONCURRENT_ALLOCATIONS + 1),
        Err(ColdLaunchError::InvalidAllocationBound)
    ));
    assert!(matches!(
        ColdLaunchOrchestrator::new(0),
        Err(ColdLaunchError::InvalidAllocationBound)
    ));
}
