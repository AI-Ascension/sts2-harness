// SPDX-License-Identifier: MIT

use super::compiler::CompiledWorkflow;
use super::definition::{AdaptiveRegionConfig, NodeDefinition, WorkflowMode};
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
}

impl<E: DynamicExecutorPort> DynamicRuntime<E> {
    pub fn new(workflow: CompiledWorkflow, executor: E) -> Result<Self, RuntimeFault> {
        if workflow.definition().mode != WorkflowMode::Dynamic {
            return Err(RuntimeFault::InvalidState);
        }
        Ok(Self {
            runtime: StrictRuntime::new(workflow)?,
            executor,
        })
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
