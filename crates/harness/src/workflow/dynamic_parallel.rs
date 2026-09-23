// SPDX-License-Identifier: MIT

//! Bounded, dependency-aware execution of an admitted dynamic analysis plan.
//!
//! Independent `Analyze` nodes are dispatched while fewer than `ParallelCap`
//! branches are in flight; the cap is enforced by this owner loop, never by the
//! executor. A node is dispatched only after every declared input (edge
//! predecessor or `Decide` input) has an outcome, and only if all of them
//! settled. Results are joined by node identity, so the joined ordering and
//! `join_digest` do not depend on completion timing.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc;
use std::thread;

use super::dynamic::{
    AnalysisFault, DynamicNode, DynamicNodeKind, DynamicPlan, DynamicPlanError,
    ParallelAnalysisExecutor,
};
use super::dynamic_join::{BranchOutcome, JoinedResult, ParallelCap, join_digest};
use super::ids::{NodeId, OperationRef};
use super::values::AnalysisValue;

/// Owner-loop decision for one ready node.
pub(crate) enum Admission {
    /// The node may be dispatched.
    Dispatch,
    /// The node is settled without dispatch, with this outcome.
    Refused(BranchOutcome),
}

/// Per-dispatch admission asked by the owner loop before a branch is spawned.
///
/// The plain bounded route uses [`NoAdmission`], which always dispatches. The
/// reserved route (`execute_plan_bounded_reserved`) supplies a budget-backed
/// admission that reserves aggregate budget before dispatch and refuses to
/// re-dispatch a branch whose reservation records a possible earlier provider
/// write.
pub(crate) trait DispatchAdmission {
    fn admit(&self, node: &NodeId) -> Admission;

    /// Record the outcome a dispatched branch reported, before it is joined.
    fn record(&self, node: &NodeId, outcome: &BranchOutcome);

    /// Whether the owner has been asked to cancel the run.
    fn cancelled(&self) -> bool {
        false
    }

    /// Called once when the run was cancelled, with the branches left in flight.
    fn on_cancel(&self, _in_flight: &BTreeSet<NodeId>) {}
}

struct NoAdmission;

impl DispatchAdmission for NoAdmission {
    fn admit(&self, _node: &NodeId) -> Admission {
        Admission::Dispatch
    }

    fn record(&self, _node: &NodeId, _outcome: &BranchOutcome) {}
}

struct Scheduled<'p> {
    node: &'p DynamicNode,
    inputs: BTreeSet<NodeId>,
    dependents: Vec<NodeId>,
}

struct JoinState<'p> {
    graph: BTreeMap<NodeId, Scheduled<'p>>,
    remaining: BTreeMap<NodeId, usize>,
    ready: BTreeSet<NodeId>,
    outcomes: BTreeMap<NodeId, BranchOutcome>,
}

impl<'p> JoinState<'p> {
    fn new(plan: &'p DynamicPlan) -> Result<Self, DynamicPlanError> {
        let mut graph: BTreeMap<NodeId, Scheduled<'p>> = BTreeMap::new();
        for node in &plan.nodes {
            let inputs = match &node.kind {
                DynamicNodeKind::Analyze { .. } => BTreeSet::new(),
                DynamicNodeKind::Decide { inputs, .. } => inputs.iter().cloned().collect(),
            };
            let scheduled = Scheduled {
                node,
                inputs,
                dependents: Vec::new(),
            };
            if graph.insert(node.id.clone(), scheduled).is_some() {
                return Err(DynamicPlanError::DuplicateIdentifier);
            }
        }
        for edge in &plan.edges {
            graph
                .get_mut(&edge.to)
                .ok_or(DynamicPlanError::MissingReference)?
                .inputs
                .insert(edge.from.clone());
        }
        let mut dependents: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
        for (id, scheduled) in &graph {
            for input in &scheduled.inputs {
                if !graph.contains_key(input) {
                    return Err(DynamicPlanError::MissingReference);
                }
                dependents
                    .entry(input.clone())
                    .or_default()
                    .push(id.clone());
            }
        }
        for (id, list) in dependents {
            if let Some(scheduled) = graph.get_mut(&id) {
                scheduled.dependents = list;
            }
        }
        let remaining: BTreeMap<NodeId, usize> = graph
            .iter()
            .map(|(id, scheduled)| (id.clone(), scheduled.inputs.len()))
            .collect();
        let ready = remaining
            .iter()
            .filter(|(_, count)| **count == 0)
            .map(|(id, _)| id.clone())
            .collect();
        Ok(Self {
            graph,
            remaining,
            ready,
            outcomes: BTreeMap::new(),
        })
    }

    fn next_ready(&mut self) -> Option<NodeId> {
        self.ready.pop_first()
    }

    /// Declared inputs of `id`, or the policy outcome when any of them did not settle.
    fn gather_inputs(
        &self,
        id: &NodeId,
    ) -> Result<(&'p DynamicNode, BTreeMap<NodeId, AnalysisValue>), BranchOutcome> {
        let Some(scheduled) = self.graph.get(id) else {
            return Err(BranchOutcome::Failed(DynamicPlanError::MissingReference));
        };
        let mut inputs = BTreeMap::new();
        for input in &scheduled.inputs {
            match self
                .outcomes
                .get(input)
                .and_then(BranchOutcome::settled_value)
            {
                Some(value) => {
                    inputs.insert(input.clone(), value.clone());
                }
                None => return Err(BranchOutcome::Failed(DynamicPlanError::UnsettledInput)),
            }
        }
        Ok((scheduled.node, inputs))
    }

    fn settle(&mut self, id: NodeId, outcome: BranchOutcome) {
        if let Some(scheduled) = self.graph.get(&id) {
            for dependent in &scheduled.dependents {
                if let Some(count) = self.remaining.get_mut(dependent) {
                    *count = count.saturating_sub(1);
                    if *count == 0 {
                        self.ready.insert(dependent.clone());
                    }
                }
            }
        }
        self.outcomes.insert(id, outcome);
    }
}

fn run_branch<A: ParallelAnalysisExecutor>(
    executor: &A,
    node: &DynamicNode,
    inputs: &BTreeMap<NodeId, AnalysisValue>,
) -> BranchOutcome {
    let (operation, context) = match &node.kind {
        DynamicNodeKind::Analyze {
            operation_ref,
            context_ref,
        } => (operation_ref.clone(), context_ref),
        DynamicNodeKind::Decide {
            decision_profile_ref,
            context_ref,
            ..
        } => match OperationRef::new(decision_profile_ref.as_str()) {
            Ok(operation) => (operation, context_ref),
            Err(_) => return BranchOutcome::Failed(DynamicPlanError::InvalidPlan),
        },
    };
    match executor.analyze(&operation, context, inputs) {
        Ok(value) => BranchOutcome::Settled(value),
        Err(AnalysisFault::Failed(error)) => BranchOutcome::Failed(error),
        Err(AnalysisFault::Unknown) => BranchOutcome::Unknown,
    }
}

/// Executes a validated plan with at most `cap` branches in flight.
///
/// Ready nodes are dispatched in node-identity order; a node whose declared
/// input failed or is unknown is never dispatched and is recorded as
/// `Failed(UnsettledInput)`. The join is keyed by `NodeId`, so `outcomes` and
/// `join_digest` are identical for every completion order. With
/// `ParallelCap::SERIAL` the settled values equal the serial `execute_plan`
/// result for a pure executor.
pub fn execute_plan_bounded<A: ParallelAnalysisExecutor>(
    plan: &DynamicPlan,
    cap: ParallelCap,
    executor: &A,
) -> Result<JoinedResult, DynamicPlanError> {
    drive(plan, cap, executor, &NoAdmission)
}

/// Owner loop shared by the plain and budget-reserved bounded routes.
///
/// Every dispatch first asks `admission` to admit the node, which lets the
/// reserved route reserve aggregate budget before a branch can spawn. When the
/// admission is cancelled the loop stops dispatching, tells the admission which
/// branches were left in flight, and reports `Cancelled`.
pub(crate) fn drive<A: ParallelAnalysisExecutor>(
    plan: &DynamicPlan,
    cap: ParallelCap,
    executor: &A,
    admission: &dyn DispatchAdmission,
) -> Result<JoinedResult, DynamicPlanError> {
    let mut state = JoinState::new(plan)?;
    let (sender, receiver) = mpsc::channel::<(NodeId, BranchOutcome)>();
    let mut cancelled = false;
    thread::scope(|scope| -> Result<(), DynamicPlanError> {
        let mut in_flight = 0usize;
        let mut in_flight_ids: BTreeSet<NodeId> = BTreeSet::new();
        loop {
            while in_flight < cap.get() {
                if admission.cancelled() {
                    cancelled = true;
                    break;
                }
                let Some(id) = state.next_ready() else { break };
                match state.gather_inputs(&id) {
                    Err(outcome) => state.settle(id, outcome),
                    Ok((node, inputs)) => match admission.admit(&node.id) {
                        Admission::Dispatch => {
                            let sender = sender.clone();
                            scope.spawn(move || {
                                // An executor is caller-supplied, so a branch may unwind. Catch it
                                // here: every spawned branch must report exactly once, or the owner
                                // loop below would wait forever for a message that never arrives.
                                let outcome =
                                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                        run_branch(executor, node, &inputs)
                                    }))
                                    .unwrap_or(BranchOutcome::Failed(DynamicPlanError::BranchLost));
                                let _ = sender.send((node.id.clone(), outcome));
                            });
                            in_flight = in_flight.saturating_add(1);
                            in_flight_ids.insert(node.id.clone());
                        }
                        Admission::Refused(outcome) => state.settle(id, outcome),
                    },
                }
            }
            if cancelled {
                break;
            }
            if in_flight == 0 {
                return Ok(());
            }
            let (id, outcome) = receiver.recv().map_err(|_| DynamicPlanError::BranchLost)?;
            in_flight = in_flight.saturating_sub(1);
            admission.record(&id, &outcome);
            in_flight_ids.remove(&id);
            state.settle(id, outcome);
        }
        admission.on_cancel(&in_flight_ids);
        Ok(())
    })?;
    if cancelled {
        return Err(DynamicPlanError::Cancelled);
    }
    if state.outcomes.len() != state.graph.len() {
        return Err(DynamicPlanError::Cycle);
    }
    let plan_digest = plan.digest()?;
    let join_digest = join_digest(&plan_digest, &state.outcomes)?;
    Ok(JoinedResult {
        plan_digest,
        outcomes: state.outcomes,
        join_digest,
    })
}
