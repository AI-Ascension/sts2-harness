// SPDX-License-Identifier: MIT

//! Deterministic ownership seams for the Linux verifier launch registry.

use super::{LaunchRegistry, RegistryError};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::thread;

const STARTING: u8 = 0;
const ALIVE: u8 = 1;
const UNKNOWN: u8 = 2;
const EXITED: u8 = 3;

struct Probe {
    phase: AtomicU8,
}

impl Probe {
    fn new(phase: u8) -> Arc<Self> {
        Arc::new(Self {
            phase: AtomicU8::new(phase),
        })
    }
}

fn reserve<'a>(
    registry: &'a LaunchRegistry<Probe>,
    owner: &Arc<Probe>,
) -> Result<super::LaunchReservation<'a, Probe>, RegistryError> {
    registry.reserve(owner, |probe| probe.phase.load(Ordering::Acquire) == EXITED)
}

fn reserve_for_test<'a>(
    registry: &'a LaunchRegistry<Probe>,
    owner: &Arc<Probe>,
) -> Option<super::LaunchReservation<'a, Probe>> {
    let result = reserve(registry, owner);
    assert!(result.is_ok(), "reservation unexpectedly failed");
    result.ok()
}

fn current(registry: &LaunchRegistry<Probe>) -> Option<Arc<Probe>> {
    let result = registry.current();
    assert!(result.is_ok(), "registry unexpectedly unavailable");
    result.ok().flatten()
}

#[test]
fn registry_mutex_contention_refuses_without_losing_candidate() {
    let registry = Arc::new(LaunchRegistry::new());
    let first = Probe::new(ALIVE);
    let owner_guard = registry.owner.lock();
    assert!(owner_guard.is_ok(), "test registry mutex poisoned");
    let Ok(_owner_guard) = owner_guard else {
        return;
    };
    let registry_thread = Arc::clone(&registry);
    let second = Probe::new(ALIVE);
    let result = thread::spawn(move || reserve(&registry_thread, &second).is_ok()).join();
    assert!(result.is_ok(), "contention probe thread panicked");
    let Ok(result) = result else {
        return;
    };
    assert!(!result);
    drop(_owner_guard);
    let reservation = reserve_for_test(&registry, &first);
    assert!(reservation.is_some());
    let Some(reservation) = reservation else {
        return;
    };
    drop(reservation);
    assert!(registry.current().is_ok_and(|owner| owner.is_some()));
}

#[test]
fn starting_owner_blocks_replacement_after_launch_token_releases() {
    let registry = LaunchRegistry::new();
    let starting = Probe::new(STARTING);
    let reservation = reserve_for_test(&registry, &starting);
    assert!(reservation.is_some());
    let Some(reservation) = reservation else {
        return;
    };
    drop(reservation);
    let replacement = Probe::new(ALIVE);
    assert!(matches!(
        reserve(&registry, &replacement),
        Err(RegistryError::Occupied)
    ));
    let current = current(&registry);
    assert!(current.is_some());
    let Some(current) = current else {
        return;
    };
    assert!(Arc::ptr_eq(&current, &starting));
}

#[test]
fn dropped_candidate_arc_is_still_retained_by_registry() {
    let registry = LaunchRegistry::new();
    let owner = Probe::new(ALIVE);
    let reservation = reserve_for_test(&registry, &owner);
    assert!(reservation.is_some());
    let Some(reservation) = reservation else {
        return;
    };
    drop(reservation);
    let weak = Arc::downgrade(&owner);
    drop(owner);
    let retained = current(&registry);
    assert!(retained.is_some());
    let Some(retained) = retained else {
        return;
    };
    assert!(weak.upgrade().is_some());
    assert_eq!(retained.phase.load(Ordering::Acquire), ALIVE);
}

#[test]
fn wait_error_owner_remains_retained_and_refuses_replacement() {
    let registry = LaunchRegistry::new();
    let uncertain = Probe::new(UNKNOWN);
    let reservation = reserve_for_test(&registry, &uncertain);
    assert!(reservation.is_some());
    let Some(reservation) = reservation else {
        return;
    };
    drop(reservation);
    let replacement = Probe::new(ALIVE);
    assert!(matches!(
        reserve(&registry, &replacement),
        Err(RegistryError::Occupied)
    ));
    let current = current(&registry);
    assert!(current.is_some());
    let Some(current) = current else {
        return;
    };
    assert!(Arc::ptr_eq(&current, &uncertain));
}

#[test]
fn two_constructors_cannot_both_hold_launch_reservations() {
    let registry = Arc::new(LaunchRegistry::new());
    let first = Probe::new(ALIVE);
    let first_reservation = reserve_for_test(&registry, &first);
    assert!(first_reservation.is_some());
    let Some(first_reservation) = first_reservation else {
        return;
    };
    let registry_thread = Arc::clone(&registry);
    let second = Probe::new(ALIVE);
    let second_result = thread::spawn(move || reserve(&registry_thread, &second).is_ok()).join();
    assert!(second_result.is_ok(), "second constructor thread panicked");
    let Ok(second_result) = second_result else {
        return;
    };
    assert!(!second_result);
    drop(first_reservation);
}

#[test]
fn active_owner_is_not_cleaned_when_replacement_is_attempted() {
    let registry = LaunchRegistry::new();
    let active = Probe::new(ALIVE);
    let reservation = reserve_for_test(&registry, &active);
    assert!(reservation.is_some());
    let Some(reservation) = reservation else {
        return;
    };
    drop(reservation);

    let replacement = Probe::new(EXITED);
    assert!(matches!(
        reserve(&registry, &replacement),
        Err(RegistryError::Occupied)
    ));
    assert_eq!(active.phase.load(Ordering::Acquire), ALIVE);
    let current = current(&registry);
    assert!(current.is_some());
    let Some(current) = current else {
        return;
    };
    assert!(Arc::ptr_eq(&current, &active));
}
