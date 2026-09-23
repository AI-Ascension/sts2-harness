// SPDX-License-Identifier: MIT

//! Atomic aggregate provider-budget reservation for the bounded analysis route.
//!
//! Every branch is a *possible provider write* — a paid inference the owner may
//! issue — so the owner reserves budget before a branch may dispatch and the
//! aggregate never oversubscribes the owner's limit, even when branches race.
//!
//! [`ParallelBudget`] is the narrow seam: the scheduler asks the port only to
//! reserve, report, or restore a reservation and never invents a provider.
//! [`BranchBudgetLedger`] mirrors the admission-before-reservation discipline of
//! [`super::provider::BudgetLedger`]: reservations are keyed by a stable branch
//! identity, a repeated reservation for the same identity and cost is
//! idempotent, and a cancelled in-flight reservation is retained, never refunded.
//! Cancellation and restart therefore refuse to re-dispatch a branch that may
//! already have inferred.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use super::dynamic::{DynamicPlan, DynamicPlanError, ParallelAnalysisExecutor};
use super::dynamic_join::{BranchOutcome, JoinedResult, ParallelCap};
use super::dynamic_parallel::{Admission, DispatchAdmission, drive};
use super::ids::{Digest, NodeId};
use super::provider::{BudgetError, ReservationState};

/// Upper bound accepted for a branch reservation.
pub const MAX_BUDGET_UNITS: u64 = 2_000_000;

/// Stable identity of one reserved analysis branch.
///
/// The plan digest plus node identity keeps a reservation stable across a cancel
/// and a restart even though the owner loop rebuilds its scheduling state.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct BranchBudgetKey {
    plan_digest: Digest,
    node: NodeId,
}

impl BranchBudgetKey {
    #[must_use]
    pub fn new(plan_digest: Digest, node: NodeId) -> Self {
        Self { plan_digest, node }
    }

    #[must_use]
    pub fn node(&self) -> &NodeId {
        &self.node
    }
}

/// One branch's reservation: units held, units realized, and its state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchReservation {
    pub key: BranchBudgetKey,
    pub reserved_units: u64,
    pub actual_units: Option<u64>,
    pub state: ReservationState,
}

/// Narrow seam for "possible provider writes".
///
/// The scheduler reserves units atomically across every branch before it can
/// dispatch. Implementations own their interior synchronization.
pub trait ParallelBudget: Send + Sync {
    /// Atomically reserve `units` for `key`; a repeated reservation for the same
    /// key and cost is idempotent and the aggregate never exceeds the limit.
    fn reserve(&self, key: &BranchBudgetKey, units: u64) -> Result<BranchReservation, BudgetError>;

    /// The reservation currently held for `key`, if any.
    fn reservation(&self, key: &BranchBudgetKey) -> Option<BranchReservation>;

    /// The aggregate units currently reserved.
    fn reserved_units(&self) -> u64;

    /// Record that a reserved branch might have reached the provider: retained.
    fn mark_unknown(&self, key: &BranchBudgetKey) -> Result<BranchReservation, BudgetError>;

    /// Record that a dispatched branch completed, realizing `actual_units`.
    fn complete(
        &self,
        key: &BranchBudgetKey,
        actual_units: u64,
    ) -> Result<BranchReservation, BudgetError>;

    /// Record a failure with no realized provider write, so restart may retry.
    fn fail(
        &self,
        key: &BranchBudgetKey,
        actual_units: Option<u64>,
    ) -> Result<BranchReservation, BudgetError>;
}

#[derive(Debug, Default)]
struct LedgerState {
    reserved: u64,
    reservations: BTreeMap<BranchBudgetKey, BranchReservation>,
}

/// Atomic aggregate budget ledger for the bounded analysis route.
///
/// One mutex protects the limit, the aggregate and every reservation, so a
/// reservation is one atomic check-then-reserve.
#[derive(Debug)]
pub struct BranchBudgetLedger {
    limit: u64,
    inner: Mutex<LedgerState>,
}

impl BranchBudgetLedger {
    pub fn new(limit: u64) -> Result<Self, BudgetError> {
        if limit == 0 || limit > MAX_BUDGET_UNITS {
            return Err(BudgetError::InvalidLimit);
        }
        Ok(Self {
            limit,
            inner: Mutex::new(LedgerState::default()),
        })
    }

    #[must_use]
    pub const fn limit(&self) -> u64 {
        self.limit
    }

    /// Recover the last committed state if a holder panicked: the aggregate is
    /// the owner's budget and must not be dropped on an unrelated unwind.
    fn lock(&self) -> MutexGuard<'_, LedgerState> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn finish(
        state: &mut LedgerState,
        key: &BranchBudgetKey,
        terminal: ReservationState,
        actual_units: Option<u64>,
    ) -> Result<BranchReservation, BudgetError> {
        let reservation = state
            .reservations
            .get_mut(key)
            .ok_or(BudgetError::Missing)?;
        if reservation.state != ReservationState::Reserved
            || actual_units.is_some_and(|units| units == 0 || units > reservation.reserved_units)
        {
            return Err(BudgetError::Conflict);
        }
        reservation.state = terminal;
        reservation.actual_units = actual_units;
        Ok(reservation.clone())
    }
}

impl ParallelBudget for BranchBudgetLedger {
    fn reserve(&self, key: &BranchBudgetKey, units: u64) -> Result<BranchReservation, BudgetError> {
        if units == 0 || units > self.limit {
            return Err(BudgetError::InvalidLimit);
        }
        let mut state = self.lock();
        if let Some(existing) = state.reservations.get(key) {
            return if existing.reserved_units == units {
                Ok(existing.clone())
            } else {
                Err(BudgetError::Duplicate)
            };
        }
        let next = state
            .reserved
            .checked_add(units)
            .ok_or(BudgetError::Capacity)?;
        if next > self.limit {
            return Err(BudgetError::Capacity);
        }
        let reservation = BranchReservation {
            key: key.clone(),
            reserved_units: units,
            actual_units: None,
            state: ReservationState::Reserved,
        };
        state.reserved = next;
        state.reservations.insert(key.clone(), reservation.clone());
        Ok(reservation)
    }

    fn reservation(&self, key: &BranchBudgetKey) -> Option<BranchReservation> {
        self.lock().reservations.get(key).cloned()
    }

    fn reserved_units(&self) -> u64 {
        self.lock().reserved
    }

    fn mark_unknown(&self, key: &BranchBudgetKey) -> Result<BranchReservation, BudgetError> {
        Self::finish(&mut self.lock(), key, ReservationState::Unknown, None)
    }

    fn complete(
        &self,
        key: &BranchBudgetKey,
        actual_units: u64,
    ) -> Result<BranchReservation, BudgetError> {
        Self::finish(
            &mut self.lock(),
            key,
            ReservationState::Completed,
            Some(actual_units),
        )
    }

    fn fail(
        &self,
        key: &BranchBudgetKey,
        actual_units: Option<u64>,
    ) -> Result<BranchReservation, BudgetError> {
        Self::finish(
            &mut self.lock(),
            key,
            ReservationState::Failed,
            actual_units,
        )
    }
}

/// Cooperative cancellation observed by the owner loop between dispatches.
pub trait CancelSignal: Send + Sync {
    fn is_cancelled(&self) -> bool;
}

/// Cancellation signal an owner or test trips from another thread.
#[derive(Debug, Default)]
pub struct CancelFlag(AtomicBool);

impl CancelFlag {
    #[must_use]
    pub fn new() -> Self {
        Self(AtomicBool::new(false))
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    #[must_use]
    pub fn is_set(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

impl CancelSignal for CancelFlag {
    fn is_cancelled(&self) -> bool {
        self.is_set()
    }
}

/// Owner-loop admission that reserves aggregate budget before every dispatch.
///
/// A branch with a held, unknown, or completed reservation records a possible
/// earlier provider write: it is never re-dispatched or re-reserved. Only a
/// branch whose reservation is a definite failure may be retried, and that retry
/// re-reserves idempotently.
pub(crate) struct BudgetAdmission<'a> {
    budget: &'a dyn ParallelBudget,
    cancel: &'a dyn CancelSignal,
    plan_digest: Digest,
    units_per_branch: u64,
}

impl<'a> BudgetAdmission<'a> {
    pub(crate) fn new(
        budget: &'a dyn ParallelBudget,
        cancel: &'a dyn CancelSignal,
        plan_digest: Digest,
        units_per_branch: u64,
    ) -> Self {
        Self {
            budget,
            cancel,
            plan_digest,
            units_per_branch,
        }
    }

    fn key(&self, node: &NodeId) -> BranchBudgetKey {
        BranchBudgetKey::new(self.plan_digest.clone(), node.clone())
    }
}

impl DispatchAdmission for BudgetAdmission<'_> {
    fn admit(&self, node: &NodeId) -> Admission {
        let key = self.key(node);
        if let Some(existing) = self.budget.reservation(&key) {
            if existing.state != ReservationState::Failed {
                return Admission::Refused(BranchOutcome::Unknown);
            }
        }
        match self.budget.reserve(&key, self.units_per_branch) {
            Ok(_) => Admission::Dispatch,
            Err(_) => Admission::Refused(BranchOutcome::Failed(DynamicPlanError::BudgetExhausted)),
        }
    }

    fn record(&self, node: &NodeId, outcome: &BranchOutcome) {
        let key = self.key(node);
        let _reported = match outcome {
            BranchOutcome::Settled(_) => self.budget.complete(&key, self.units_per_branch),
            BranchOutcome::Failed(_) => self.budget.fail(&key, None),
            BranchOutcome::Unknown => self.budget.mark_unknown(&key),
        };
    }

    fn cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    fn on_cancel(&self, in_flight: &BTreeSet<NodeId>) {
        for node in in_flight {
            let _retained = self.budget.mark_unknown(&self.key(node));
        }
    }
}

/// Executes a validated plan like
/// [`execute_plan_bounded`](super::dynamic_parallel::execute_plan_bounded), but
/// gates every dispatch on an atomic aggregate budget reservation.
///
/// A branch the ledger cannot reserve is recorded as `Failed(BudgetExhausted)`
/// and never dispatched, so racing branches cannot oversubscribe the owner's
/// limit. A cancel signal stops new dispatches, marks the reservations left in
/// flight [`ReservationState::Unknown`] and returns
/// [`DynamicPlanError::Cancelled`]; the reservations persist in `budget`, so a
/// restart re-reserves nothing and re-dispatches no branch that may already have
/// inferred.
pub fn execute_plan_bounded_reserved<A: ParallelAnalysisExecutor>(
    plan: &DynamicPlan,
    cap: ParallelCap,
    executor: &A,
    budget: &dyn ParallelBudget,
    units_per_branch: u64,
    cancel: &dyn CancelSignal,
) -> Result<JoinedResult, DynamicPlanError> {
    if units_per_branch == 0 || units_per_branch > MAX_BUDGET_UNITS {
        return Err(DynamicPlanError::Capacity);
    }
    let admission = BudgetAdmission::new(budget, cancel, plan.digest()?, units_per_branch);
    drive(plan, cap, executor, &admission)
}
