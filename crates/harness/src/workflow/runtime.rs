// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use super::compiler::{CompiledGraph, CompiledWorkflow};
use super::definition::{EdgeOutcome, NodeDefinition, NodeKind, TerminalOutcome};
use super::guards::{GuardContext, TruthValue};
use super::ids::{GraphId, NodeId};
use super::runtime_types::{
    NodeExecutor, NodeOutcome, ReturnFrame, RuntimeContext, RuntimeEvent, RuntimeFault,
    RuntimeRunReport, RuntimeSnapshot, RuntimeStatus,
};
use super::values::TypedValue;

pub struct StrictRuntime {
    workflow: CompiledWorkflow,
    snapshot: RuntimeSnapshot,
    events: Vec<RuntimeEvent>,
}

impl StrictRuntime {
    pub fn new(workflow: CompiledWorkflow) -> Result<Self, RuntimeFault> {
        let graph = workflow
            .entry_graph()
            .map_err(|_| RuntimeFault::InvalidState)?;
        let snapshot = RuntimeSnapshot {
            semantic_digest: workflow.semantic_digest().clone(),
            graph_id: graph.id().clone(),
            node_id: graph.entry_node().clone(),
            stack: Vec::new(),
            steps: 0,
            status: RuntimeStatus::Running,
            outputs: BTreeMap::new(),
            guard_fields: BTreeMap::new(),
            loop_iterations: BTreeMap::new(),
            event_sequence: 0,
        };
        Ok(Self {
            workflow,
            snapshot,
            events: Vec::new(),
        })
    }

    pub fn from_snapshot(
        workflow: CompiledWorkflow,
        snapshot: RuntimeSnapshot,
    ) -> Result<Self, RuntimeFault> {
        if snapshot.semantic_digest != *workflow.semantic_digest()
            || snapshot.steps > workflow.definition().limits.max_steps
            || snapshot.stack.len() > workflow.definition().limits.max_subworkflow_depth as usize
            || snapshot.status.terminal() && snapshot.node_id.as_str().is_empty()
        {
            return Err(RuntimeFault::InvalidState);
        }
        if workflow.graph(&snapshot.graph_id).is_none() {
            return Err(RuntimeFault::InvalidState);
        }
        for frame in &snapshot.stack {
            if workflow
                .graph(&frame.graph_id)
                .and_then(|graph| graph.node(&frame.node_id))
                .is_none()
            {
                return Err(RuntimeFault::InvalidState);
            }
        }
        if workflow
            .graph(&snapshot.graph_id)
            .and_then(|graph| graph.node(&snapshot.node_id))
            .is_none()
        {
            return Err(RuntimeFault::InvalidState);
        }
        Ok(Self {
            workflow,
            snapshot,
            events: Vec::new(),
        })
    }

    #[must_use]
    pub fn status(&self) -> RuntimeStatus {
        self.snapshot.status
    }

    #[must_use]
    pub fn snapshot(&self) -> RuntimeSnapshot {
        self.snapshot.clone()
    }

    #[must_use]
    pub fn events(&self) -> &[RuntimeEvent] {
        &self.events
    }

    pub fn pause(&mut self) -> Result<(), RuntimeFault> {
        if self.snapshot.status != RuntimeStatus::Running {
            return Err(RuntimeFault::InvalidState);
        }
        self.snapshot.status = RuntimeStatus::Paused;
        Ok(())
    }

    pub fn resume(&mut self) -> Result<(), RuntimeFault> {
        if self.snapshot.status != RuntimeStatus::Paused {
            return Err(RuntimeFault::InvalidState);
        }
        self.snapshot.status = RuntimeStatus::Running;
        Ok(())
    }

    pub fn step<E: NodeExecutor>(
        &mut self,
        executor: &mut E,
    ) -> Result<RuntimeStatus, RuntimeFault> {
        if self.snapshot.status != RuntimeStatus::Running {
            return Err(RuntimeFault::InvalidState);
        }
        if self.snapshot.steps >= self.workflow.definition().limits.max_steps {
            self.snapshot.status = RuntimeStatus::NeedsOperator;
            return Err(RuntimeFault::BudgetExceeded);
        }
        let graph = self.current_graph()?.clone();
        let node = graph
            .node(&self.snapshot.node_id)
            .ok_or(RuntimeFault::InvalidState)?
            .clone();
        match &node {
            NodeDefinition::Terminal { config, .. } => {
                self.snapshot.steps = self.snapshot.steps.saturating_add(1);
                self.finish_terminal(&graph, config.outcome)?;
                self.record_event(None, Some(config.outcome))?;
            }
            NodeDefinition::Loop { config, .. } => {
                let outcome = self.step_loop(
                    &graph,
                    config.body_graph.clone(),
                    config.exit_guard_ref.clone(),
                    config.max_iterations,
                )?;
                self.snapshot.steps = self.snapshot.steps.saturating_add(1);
                self.record_event(Some(outcome), None)?;
            }
            _ => {
                let context = RuntimeContext {
                    graph_id: graph.id().clone(),
                    node_id: self.snapshot.node_id.clone(),
                    step: self.snapshot.steps,
                    outputs: self.snapshot.outputs.clone(),
                    guard_fields: self.snapshot.guard_fields.clone(),
                };
                let result = match executor.execute(&node, &context) {
                    Ok(result) => result,
                    Err(error) => {
                        if error == RuntimeFault::UnknownEffect {
                            self.snapshot.status = RuntimeStatus::NeedsOperator;
                        }
                        return Err(error);
                    }
                };
                self.accept_output(&node, &result)?;
                self.snapshot
                    .guard_fields
                    .extend(result.guard_fields.clone());
                self.snapshot.steps = self.snapshot.steps.saturating_add(1);
                let next = self.select_edge(&graph, result.outcome)?;
                self.snapshot.graph_id = graph.id().clone();
                self.snapshot.node_id = next;
                self.record_event(Some(result.outcome), None)?;
            }
        }
        Ok(self.snapshot.status)
    }

    pub fn run_to_completion<E: NodeExecutor>(
        &mut self,
        executor: &mut E,
    ) -> Result<RuntimeRunReport, RuntimeFault> {
        while self.snapshot.status == RuntimeStatus::Running {
            self.step(executor)?;
        }
        Ok(RuntimeRunReport {
            status: self.snapshot.status,
            terminal: self.events.iter().rev().find_map(|event| event.terminal),
            steps: self.snapshot.steps,
            events: self.events.clone(),
            snapshot: self.snapshot(),
        })
    }

    fn current_graph(&self) -> Result<&CompiledGraph, RuntimeFault> {
        self.workflow
            .graph(&self.snapshot.graph_id)
            .ok_or(RuntimeFault::InvalidState)
    }

    fn accept_output(
        &mut self,
        node: &NodeDefinition,
        result: &NodeOutcome,
    ) -> Result<(), RuntimeFault> {
        if result.outcome == EdgeOutcome::Ok
            && !matches!(result.value, TypedValue::Unknown | TypedValue::Unavailable)
        {
            let expected = node
                .output_type(output_name(node))
                .ok_or(RuntimeFault::TypeMismatch)?;
            if expected != result.value.value_type() {
                return Err(RuntimeFault::TypeMismatch);
            }
        }
        let key = format!("{}/{}", node.id(), output_name(node));
        self.snapshot.outputs.insert(key, result.value.clone());
        Ok(())
    }

    fn select_edge(
        &self,
        graph: &CompiledGraph,
        outcome: EdgeOutcome,
    ) -> Result<NodeId, RuntimeFault> {
        let mut unknown_guard = false;
        for edge in graph.edges_from(&self.snapshot.node_id) {
            if edge.on != outcome {
                continue;
            }
            if let Some(guard) = edge.guard_ref.as_ref() {
                let expression = graph.guard(guard).ok_or(RuntimeFault::InvalidState)?;
                let mut context = GuardContext::new();
                for (field, value) in &self.snapshot.guard_fields {
                    context.insert(field.clone(), value.clone());
                }
                match context
                    .evaluate(expression)
                    .map_err(|_| RuntimeFault::GuardFailure)?
                {
                    TruthValue::True => return Ok(edge.to.clone()),
                    TruthValue::False => continue,
                    TruthValue::Unknown => unknown_guard = true,
                }
            } else {
                return Ok(edge.to.clone());
            }
        }
        if unknown_guard {
            for edge in graph.edges_from(&self.snapshot.node_id) {
                if edge.on == EdgeOutcome::Unknown && edge.guard_ref.is_none() {
                    return Ok(edge.to.clone());
                }
            }
        }
        Err(RuntimeFault::MissingRoute)
    }

    fn step_loop(
        &mut self,
        graph: &CompiledGraph,
        body_graph: GraphId,
        guard_ref: super::ids::GuardId,
        max_iterations: u64,
    ) -> Result<EdgeOutcome, RuntimeFault> {
        let expression = graph.guard(&guard_ref).ok_or(RuntimeFault::InvalidState)?;
        let mut context = GuardContext::new();
        for (field, value) in &self.snapshot.guard_fields {
            context.insert(field.clone(), value.clone());
        }
        let guard = context
            .evaluate(expression)
            .map_err(|_| RuntimeFault::GuardFailure)?;
        match guard {
            TruthValue::True => {
                self.snapshot.loop_iterations.remove(&loop_key(graph));
                self.snapshot.node_id = self.select_edge(graph, EdgeOutcome::True)?;
                Ok(EdgeOutcome::True)
            }
            TruthValue::Unknown => {
                self.snapshot.node_id = self.select_edge(graph, EdgeOutcome::Unknown)?;
                Ok(EdgeOutcome::Unknown)
            }
            TruthValue::False => {
                let key = loop_key(graph);
                let count = self.snapshot.loop_iterations.entry(key).or_insert(0);
                if *count >= max_iterations {
                    self.snapshot.status = RuntimeStatus::NeedsOperator;
                    return Err(RuntimeFault::BudgetExceeded);
                }
                *count = count.saturating_add(1);
                if self.snapshot.stack.len()
                    >= self.workflow.definition().limits.max_subworkflow_depth as usize
                {
                    self.snapshot.status = RuntimeStatus::NeedsOperator;
                    return Err(RuntimeFault::BudgetExceeded);
                }
                let continuation = self.select_edge(graph, EdgeOutcome::True)?;
                let body = self
                    .workflow
                    .graph(&body_graph)
                    .ok_or(RuntimeFault::InvalidState)?;
                self.snapshot.stack.push(ReturnFrame {
                    graph_id: graph.id().clone(),
                    node_id: continuation,
                });
                self.snapshot.graph_id = body.id().clone();
                self.snapshot.node_id = body.entry_node().clone();
                Ok(EdgeOutcome::False)
            }
        }
    }

    fn finish_terminal(
        &mut self,
        graph: &CompiledGraph,
        outcome: TerminalOutcome,
    ) -> Result<(), RuntimeFault> {
        if let Some(frame) = self.snapshot.stack.pop() {
            self.snapshot.graph_id = frame.graph_id;
            self.snapshot.node_id = frame.node_id;
            return Ok(());
        }
        self.snapshot.status = match outcome {
            TerminalOutcome::Completed => RuntimeStatus::Completed,
            TerminalOutcome::Failed => RuntimeStatus::Failed,
            TerminalOutcome::NeedsOperator => RuntimeStatus::NeedsOperator,
        };
        self.snapshot.graph_id = graph.id().clone();
        Ok(())
    }

    fn record_event(
        &mut self,
        outcome: Option<EdgeOutcome>,
        terminal: Option<TerminalOutcome>,
    ) -> Result<(), RuntimeFault> {
        self.snapshot.event_sequence = self
            .snapshot
            .event_sequence
            .checked_add(1)
            .ok_or(RuntimeFault::BudgetExceeded)?;
        self.events.push(RuntimeEvent {
            sequence: self.snapshot.event_sequence,
            graph_id: self.snapshot.graph_id.clone(),
            node_id: self.snapshot.node_id.clone(),
            outcome,
            status: self.snapshot.status,
            terminal,
        });
        Ok(())
    }
}

fn output_name(node: &NodeDefinition) -> &'static str {
    match node.kind() {
        NodeKind::Observe | NodeKind::AwaitStability => "observation",
        NodeKind::Route => "route",
        NodeKind::Analyze => "analysis",
        NodeKind::Decide | NodeKind::AdaptiveRegion => "proposal",
        NodeKind::ExecuteAction | NodeKind::Loop | NodeKind::Checkpoint | NodeKind::Pause => {
            "result"
        }
        NodeKind::Subworkflow => "selection",
        NodeKind::EmitArtifact => "artifact",
        NodeKind::Terminal => "terminal",
    }
}

fn loop_key(graph: &CompiledGraph) -> String {
    format!("{}:{}", graph.id(), graph.entry_node())
}
