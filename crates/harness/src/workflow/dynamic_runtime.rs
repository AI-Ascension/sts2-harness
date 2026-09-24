// SPDX-License-Identifier: MIT

use super::bounded_region::{BoundedAnalysisOutcome, BoundedRegionRefusal, run_bounded_region};
use super::compiler::CompiledWorkflow;
use super::definition::{AdaptiveRegionConfig, NodeDefinition, WorkflowLimits, WorkflowMode};
use super::dynamic::{DynamicPlan, ParallelAnalysisExecutor};
use super::dynamic_budget::{CancelSignal, ParallelBudget};
use super::runtime::StrictRuntime;
use super::runtime_types::{
    NodeExecutor, NodeOutcome, RuntimeContext, RuntimeFault, RuntimeRunReport, RuntimeSnapshot,
    RuntimeStatus,
};

pub trait DynamicExecutorPort: NodeExecutor {
    fn execute_adaptive(
        &mut self,
        config: &AdaptiveRegionConfig,
        context: &RuntimeContext,
    ) -> Result<NodeOutcome, RuntimeFault>;
}

pub struct DynamicRuntime<E> {
    runtime: StrictRuntime,
    executor: E,
    limits: WorkflowLimits,
    last_bounded: Option<BoundedAnalysisOutcome>,
}

impl<E: DynamicExecutorPort> DynamicRuntime<E> {
    pub fn new(workflow: CompiledWorkflow, executor: E) -> Result<Self, RuntimeFault> {
        if workflow.definition().mode != WorkflowMode::Dynamic {
            return Err(RuntimeFault::InvalidState);
        }
        let limits = workflow.definition().limits.clone();
        Ok(Self {
            runtime: StrictRuntime::new(workflow)?,
            executor,
            limits,
            last_bounded: None,
        })
    }

    /// Executes one declared analysis region on the budget-reserved bounded route.
    ///
    /// Admission runs first, so a region whose cap is outside the admitted range,
    /// that admits no operation, or whose plan is invalid is refused with a typed
    /// [`BoundedRegionRefusal`] before any branch can be dispatched. On success the
    /// owner loop's actual per-branch states are retained for consumers, and the
    /// outcome is also returned directly.
    ///
    /// The dispatched work is analysis only: a [`DynamicPlan`] cannot name a
    /// mutating node kind, so this route cannot reach a concurrent game mutation.
    pub fn execute_bounded_region<A: ParallelAnalysisExecutor>(
        &mut self,
        plan: &DynamicPlan,
        region: &AdaptiveRegionConfig,
        executor: &A,
        budget: &dyn ParallelBudget,
        units_per_branch: u64,
        cancel: &dyn CancelSignal,
    ) -> Result<BoundedAnalysisOutcome, BoundedRegionRefusal> {
        let (outcome, _joined) = run_bounded_region(
            plan,
            region,
            &self.limits,
            executor,
            budget,
            units_per_branch,
            cancel,
        )?;
        self.last_bounded = Some(outcome.clone());
        Ok(outcome)
    }

    /// The most recent bounded-region report, or `None` if none ran.
    #[must_use]
    pub fn last_bounded_outcome(&self) -> Option<&BoundedAnalysisOutcome> {
        self.last_bounded.as_ref()
    }

    pub fn step(&mut self) -> Result<RuntimeStatus, RuntimeFault> {
        let mut dispatcher = Dispatch {
            executor: &mut self.executor,
        };
        self.runtime.step(&mut dispatcher)
    }

    pub fn run_to_completion(&mut self) -> Result<RuntimeRunReport, RuntimeFault> {
        while self.runtime.status() == RuntimeStatus::Running {
            self.step()?;
        }
        Ok(RuntimeRunReport {
            status: self.runtime.status(),
            terminal: self
                .runtime
                .events()
                .iter()
                .rev()
                .find_map(|event| event.terminal),
            steps: self.runtime.snapshot().steps,
            events: self.runtime.events().to_vec(),
            snapshot: self.runtime.snapshot(),
        })
    }

    #[must_use]
    pub fn status(&self) -> RuntimeStatus {
        self.runtime.status()
    }

    #[must_use]
    pub fn snapshot(&self) -> RuntimeSnapshot {
        self.runtime.snapshot()
    }

    #[must_use]
    pub fn executor(&self) -> &E {
        &self.executor
    }
}

struct Dispatch<'a, E> {
    executor: &'a mut E,
}

impl<E: DynamicExecutorPort> NodeExecutor for Dispatch<'_, E> {
    fn execute(
        &mut self,
        node: &NodeDefinition,
        context: &RuntimeContext,
    ) -> Result<NodeOutcome, RuntimeFault> {
        match node {
            NodeDefinition::AdaptiveRegion { config, .. } => {
                self.executor.execute_adaptive(config, context)
            }
            _ => self.executor.execute(node, context),
        }
    }
}
