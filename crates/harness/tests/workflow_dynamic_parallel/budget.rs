// SPDX-License-Identifier: MIT

//! AC4: atomic aggregate budget reservation, cancellation and restart.
//!
//! The owner loop admits a branch *before* it spawns it, so the ledger's atomic
//! check-then-reserve is what stops two racing branches from oversubscribing the
//! owner's budget. These tests exercise the ledger directly under a barrier race
//! and exercise the scheduler boundary through `execute_plan_bounded_reserved`.

#![allow(clippy::expect_used)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Barrier, Mutex};
use std::thread;

use sts2_harness::workflow::{
    BranchBudgetKey, BranchBudgetLedger, BranchOutcome, BudgetError, CancelFlag, CancelSignal,
    Digest, DynamicPlanError, ParallelBudget, ParallelCap, ReservationState, execute_plan_bounded,
    execute_plan_bounded_reserved,
};

use super::controlled::{Controlled, analyze, node, plan};

const UNITS: u64 = 3;
const LIMIT: u64 = 6;
const BRANCHES: usize = 4;

fn digest(tag: &str) -> Digest {
    Digest::sha256(tag.as_bytes())
}

fn key(tag: &str, index: usize) -> BranchBudgetKey {
    BranchBudgetKey::new(digest(tag), node(&format!("n{index}")))
}

/// Cancellation signal that trips after `allowed` ordinary checks.
struct CancelAfter {
    allowed: usize,
    checks: AtomicUsize,
}

impl CancelAfter {
    fn new(allowed: usize) -> Self {
        Self {
            allowed,
            checks: AtomicUsize::new(0),
        }
    }
}

impl CancelSignal for CancelAfter {
    fn is_cancelled(&self) -> bool {
        self.checks.fetch_add(1, Ordering::SeqCst) >= self.allowed
    }
}

/// Negative-control ledger that performs the capacity check outside the commit.
///
/// A barrier between the read and the write makes every racer observe the same
/// remaining capacity, so this deterministically oversubscribes where the real
/// [`BranchBudgetLedger`] cannot. It stands in for an implementation whose
/// aggregate accounting is not atomic.
struct NonAtomicControlLedger {
    limit: u64,
    reserved: Mutex<u64>,
    gate: Barrier,
}

impl NonAtomicControlLedger {
    fn new(limit: u64, racers: usize) -> Self {
        Self {
            limit,
            reserved: Mutex::new(0),
            gate: Barrier::new(racers),
        }
    }

    fn reserve(&self, units: u64) -> Result<(), BudgetError> {
        let seen = *self.reserved.lock().expect("control aggregate");
        self.gate.wait();
        if seen.saturating_add(units) > self.limit {
            return Err(BudgetError::Capacity);
        }
        *self.reserved.lock().expect("control aggregate") += units;
        Ok(())
    }

    fn reserved_units(&self) -> u64 {
        *self.reserved.lock().expect("control aggregate")
    }
}

#[test]
fn racing_reservations_never_oversubscribe_the_aggregate_budget() {
    let ledger = BranchBudgetLedger::new(LIMIT).expect("limit");
    let barrier = Barrier::new(BRANCHES);
    let granted = AtomicUsize::new(0);
    thread::scope(|scope| {
        for index in 0..BRANCHES {
            let ledger = &ledger;
            let barrier = &barrier;
            let granted = &granted;
            scope.spawn(move || {
                barrier.wait();
                match ledger.reserve(&key("race", index), UNITS) {
                    Ok(_) => {
                        granted.fetch_add(1, Ordering::SeqCst);
                    }
                    Err(BudgetError::Capacity) => {}
                    Err(other) => assert_eq!(other, BudgetError::Capacity, "unexpected refusal"),
                }
            });
        }
    });

    let committed = ledger.reserved_units();
    assert_eq!(committed, LIMIT, "exactly floor(limit/units) racers commit");
    assert_eq!(granted.load(Ordering::SeqCst), (LIMIT / UNITS) as usize);
    assert!(
        committed <= ledger.limit() && (BRANCHES as u64) * UNITS > committed,
        "aggregate {committed} must not reach the {BRANCHES}x{UNITS} the racers asked for"
    );
}

#[test]
fn control_without_atomic_accounting_oversubscribes_then_the_ledger_passes() {
    let control = NonAtomicControlLedger::new(LIMIT, BRANCHES);
    let refused = AtomicUsize::new(0);
    thread::scope(|scope| {
        for _ in 0..BRANCHES {
            let control = &control;
            let refused = &refused;
            scope.spawn(move || {
                if control.reserve(UNITS).is_err() {
                    refused.fetch_add(1, Ordering::SeqCst);
                }
            });
        }
    });
    assert_eq!(refused.load(Ordering::SeqCst), 0, "control refused nothing");
    assert!(
        control.reserved_units() > LIMIT,
        "control oversubscribed: {} > {LIMIT}",
        control.reserved_units()
    );

    let ledger = BranchBudgetLedger::new(LIMIT).expect("limit");
    for index in 0..BRANCHES {
        let _attempt = ledger.reserve(&key("restored", index), UNITS);
    }
    assert_eq!(
        ledger.reserved_units(),
        LIMIT,
        "the ledger respects the cap"
    );
}

#[test]
fn a_branch_that_cannot_reserve_does_not_dispatch() {
    let plan = plan(
        vec![analyze("a"), analyze("b"), analyze("c"), analyze("d")],
        Vec::new(),
    );
    let cap = ParallelCap::new(4).expect("cap");
    let ledger = BranchBudgetLedger::new(LIMIT).expect("limit");
    let executor = Controlled::default();

    let joined =
        execute_plan_bounded_reserved(&plan, cap, &executor, &ledger, UNITS, &CancelFlag::new())
            .expect("joins");

    let dispatched = executor.calls.load(Ordering::SeqCst);
    assert_eq!(
        dispatched as u64,
        LIMIT / UNITS,
        "only branches that reserved may dispatch"
    );
    assert_eq!(ledger.reserved_units(), LIMIT);
    assert_eq!(
        joined
            .outcomes
            .values()
            .filter(|outcome| matches!(outcome, BranchOutcome::Failed(_)))
            .count(),
        BRANCHES - dispatched,
        "a branch that could not reserve is settled without dispatch"
    );
    assert_eq!(
        joined
            .outcomes
            .values()
            .filter(|outcome| outcome.is_settled())
            .count(),
        dispatched
    );
}

#[test]
fn cancel_retains_the_reservation_and_restart_does_not_redispatch() {
    let plan = plan(
        vec![analyze("a"), analyze("b"), analyze("c"), analyze("d")],
        Vec::new(),
    );
    let cap = ParallelCap::new(4).expect("cap");
    let ledger = BranchBudgetLedger::new(4 * UNITS).expect("limit");
    let cancel = CancelAfter::new(1);
    let first = Controlled::default();

    let cancelled = execute_plan_bounded_reserved(&plan, cap, &first, &ledger, UNITS, &cancel);
    assert_eq!(cancelled, Err(DynamicPlanError::Cancelled));
    assert_eq!(
        first.calls.load(Ordering::SeqCst),
        1,
        "one branch was in flight"
    );
    let reserved = BranchBudgetKey::new(plan.digest().expect("digest"), node("a"));
    assert_eq!(
        ledger.reserved_units(),
        UNITS,
        "a cancelled in-flight reservation is retained, never refunded"
    );
    assert_eq!(
        ledger.reservation(&reserved).map(|entry| entry.state),
        Some(ReservationState::Unknown),
        "the branch may already have written to the provider"
    );

    let second = Controlled::default();
    let restarted =
        execute_plan_bounded_reserved(&plan, cap, &second, &ledger, UNITS, &CancelFlag::new())
            .expect("restart joins");

    let log = second.log();
    assert!(
        !log.iter().any(|line| line == "start a"),
        "the unknown branch is not re-dispatched: {log:?}"
    );
    assert_eq!(second.calls.load(Ordering::SeqCst), BRANCHES - 1);
    assert_eq!(
        restarted.outcomes.get(&node("a")),
        Some(&BranchOutcome::Unknown)
    );
    assert_eq!(
        ledger.reserved_units(),
        4 * UNITS,
        "restart added no duplicate reservation"
    );
    assert_eq!(
        ledger
            .reservation(&reserved)
            .map(|entry| entry.reserved_units),
        Some(UNITS)
    );
}

#[test]
fn a_definite_failure_retries_once_while_a_possible_write_is_never_repeated() {
    let plan = plan(
        vec![analyze("a"), analyze("b"), analyze("c"), analyze("d")],
        Vec::new(),
    );
    let cap = ParallelCap::new(4).expect("cap");
    let ledger = BranchBudgetLedger::new(4 * UNITS).expect("limit");
    let first = Controlled {
        fail_once: Mutex::new(std::collections::BTreeSet::from(["a".to_owned()])),
        ..Controlled::default()
    };
    let joined =
        execute_plan_bounded_reserved(&plan, cap, &first, &ledger, UNITS, &CancelFlag::new())
            .expect("joins");
    assert!(matches!(
        joined.outcomes.get(&node("a")),
        Some(BranchOutcome::Failed(_))
    ));
    let failed = BranchBudgetKey::new(plan.digest().expect("digest"), node("a"));
    assert_eq!(
        ledger.reservation(&failed).map(|entry| entry.state),
        Some(ReservationState::Failed)
    );

    let second = Controlled::default();
    let restarted =
        execute_plan_bounded_reserved(&plan, cap, &second, &ledger, UNITS, &CancelFlag::new())
            .expect("restart joins");

    let log = second.log();
    let started = log.iter().filter(|line| line.starts_with("start ")).count();
    assert!(log.iter().any(|line| line == "start a"), "{log:?}");
    assert_eq!(
        started, 1,
        "only the definite failure retried; completed branches are not re-inferred"
    );
    assert!(matches!(
        restarted.outcomes.get(&node("a")),
        Some(BranchOutcome::Settled(_))
    ));
    assert_eq!(ledger.reserved_units(), 4 * UNITS);
    assert_eq!(
        ledger.reservation(&failed).map(|entry| entry.actual_units),
        Some(Some(UNITS))
    );
}

#[test]
fn without_the_aggregate_reservation_every_branch_could_infer() {
    let plan = plan(
        vec![analyze("a"), analyze("b"), analyze("c"), analyze("d")],
        Vec::new(),
    );
    let cap = ParallelCap::new(4).expect("cap");

    let control = Controlled::default();
    let plain = execute_plan_bounded(&plan, cap, &control).expect("plain joins");
    let control_inference = control.calls.load(Ordering::SeqCst) as u64 * UNITS;
    assert!(
        plain.outcomes.values().all(BranchOutcome::is_settled),
        "the reservation-free route dispatches every branch"
    );
    assert!(
        control_inference > LIMIT,
        "control would oversubscribe the {LIMIT}-unit budget with {control_inference} units"
    );

    let ledger = BranchBudgetLedger::new(LIMIT).expect("limit");
    let restored = Controlled::default();
    let reserved =
        execute_plan_bounded_reserved(&plan, cap, &restored, &ledger, UNITS, &CancelFlag::new())
            .expect("reserved joins");
    assert_eq!(ledger.reserved_units(), LIMIT);
    assert!(ledger.reserved_units() <= ledger.limit());
    assert_eq!(restored.calls.load(Ordering::SeqCst) as u64, LIMIT / UNITS);
    assert!(
        reserved
            .outcomes
            .values()
            .all(|outcome| { outcome.is_settled() || matches!(outcome, BranchOutcome::Failed(_)) })
    );
}
