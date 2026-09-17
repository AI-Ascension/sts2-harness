// SPDX-License-Identifier: MIT

//! Controlled executor and plan builders for the bounded analysis route tests.

#![allow(clippy::expect_used)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use sts2_harness::workflow::{
    AnalysisFault, AnalysisValue, BoundedText, ContextId, DecisionProfileId, Digest, DynamicEdge,
    DynamicNode, DynamicNodeKind, DynamicPlan, DynamicPlanError, NodeId, OperationRef,
    ParallelAnalysisExecutor, PlanId, PlannerProfileId, PureAnalysisExecutor, RegionId, Revision,
    WorkflowLimits,
};

const RENDEZVOUS_DEADLINE: Duration = Duration::from_millis(500);
const BRANCH_WORK: Duration = Duration::from_millis(20);

pub fn node(id: &str) -> NodeId {
    NodeId::new(id).expect("node id")
}

pub fn analyze(id: &str) -> DynamicNode {
    DynamicNode {
        id: node(id),
        kind: DynamicNodeKind::Analyze {
            operation_ref: OperationRef::new(format!("map.inspect.{id}")).expect("operation"),
            context_ref: ContextId::new("context-1").expect("context"),
        },
    }
}

pub fn decide(id: &str, inputs: &[&str]) -> DynamicNode {
    DynamicNode {
        id: node(id),
        kind: DynamicNodeKind::Decide {
            decision_profile_ref: DecisionProfileId::new("decision.profile").expect("profile"),
            context_ref: ContextId::new("context-1").expect("context"),
            inputs: inputs.iter().map(|input| node(input)).collect(),
        },
    }
}

pub fn edge(from: &str, to: &str) -> DynamicEdge {
    DynamicEdge {
        from: node(from),
        to: node(to),
    }
}

pub fn plan(nodes: Vec<DynamicNode>, edges: Vec<DynamicEdge>) -> DynamicPlan {
    DynamicPlan {
        plan_id: PlanId::new("plan-1").expect("plan id"),
        region_id: RegionId::new("region-1").expect("region id"),
        planner_profile_ref: PlannerProfileId::new("planner-1").expect("planner id"),
        base_semantic_digest: Digest::sha256(b"workflow"),
        base_revision: Revision::new(1).expect("revision"),
        dependency_digest: Digest::sha256(b"inputs"),
        nodes,
        edges,
    }
}

pub fn value(code: &str, inputs: &BTreeMap<NodeId, AnalysisValue>) -> AnalysisValue {
    let joined = inputs
        .keys()
        .map(NodeId::as_str)
        .collect::<Vec<_>>()
        .join(",");
    AnalysisValue {
        code: BoundedText::new(format!("{code}<-{joined}")).expect("code"),
        fields: BTreeMap::new(),
    }
}

pub fn operation_suffix(operation: &OperationRef) -> String {
    operation
        .as_str()
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// Controlled executor: records the live in-flight count, its peak, an ordered
/// start/end log, and returns scripted faults per operation suffix.
#[derive(Default)]
pub struct Controlled {
    pub current: AtomicUsize,
    pub peak: AtomicUsize,
    pub calls: AtomicUsize,
    pub rendezvous: Option<usize>,
    pub delays: BTreeMap<String, Duration>,
    pub fail_once: Mutex<BTreeSet<String>>,
    pub unknown: BTreeSet<String>,
    pub log: Mutex<Vec<String>>,
}

impl Controlled {
    pub fn peak(&self) -> usize {
        self.peak.load(Ordering::SeqCst)
    }

    pub fn log(&self) -> Vec<String> {
        self.log.lock().map(|log| log.clone()).unwrap_or_default()
    }

    fn record(&self, entry: String) {
        if let Ok(mut log) = self.log.lock() {
            log.push(entry);
        }
    }

    fn enter(&self) {
        let live = self
            .current
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1);
        self.peak.fetch_max(live, Ordering::SeqCst);
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(target) = self.rendezvous {
            let deadline = Instant::now() + RENDEZVOUS_DEADLINE;
            while self.current.load(Ordering::SeqCst) < target && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(1));
            }
        }
    }

    fn leave(&self) {
        self.current.fetch_sub(1, Ordering::SeqCst);
    }
}

impl ParallelAnalysisExecutor for Controlled {
    fn analyze(
        &self,
        operation: &OperationRef,
        _context: &ContextId,
        inputs: &BTreeMap<NodeId, AnalysisValue>,
    ) -> Result<AnalysisValue, AnalysisFault> {
        let name = operation_suffix(operation);
        self.enter();
        self.record(format!("start {name}"));
        thread::sleep(self.delays.get(&name).copied().unwrap_or(BRANCH_WORK));
        let fails = self
            .fail_once
            .lock()
            .map(|mut set| set.remove(&name))
            .unwrap_or(false);
        let result = if fails {
            Err(AnalysisFault::Failed(DynamicPlanError::UnknownOperation))
        } else if self.unknown.contains(&name) {
            Err(AnalysisFault::Unknown)
        } else {
            Ok(value(&name, inputs))
        };
        self.record(format!("end {name}"));
        self.leave();
        result
    }
}

pub struct Serial;

/// Executor whose named branches unwind, standing in for a caller-supplied
/// implementation that panics while the owner loop is waiting on its report.
#[derive(Default)]
pub struct Panics {
    pub unwound: BTreeSet<String>,
}

impl ParallelAnalysisExecutor for Panics {
    fn analyze(
        &self,
        operation: &OperationRef,
        _context: &ContextId,
        inputs: &BTreeMap<NodeId, AnalysisValue>,
    ) -> Result<AnalysisValue, AnalysisFault> {
        let name = operation_suffix(operation);
        if self.unwound.contains(&name) {
            std::panic::resume_unwind(Box::new("synthetic branch unwind"));
        }
        Ok(value(&name, inputs))
    }
}

impl PureAnalysisExecutor for Serial {
    fn analyze(
        &mut self,
        operation: &OperationRef,
        _context: &ContextId,
        inputs: &BTreeMap<NodeId, AnalysisValue>,
    ) -> Result<AnalysisValue, DynamicPlanError> {
        Ok(value(&operation_suffix(operation), inputs))
    }
}

pub fn allowed(plan: &DynamicPlan) -> BTreeSet<OperationRef> {
    plan.nodes
        .iter()
        .filter_map(|node| match &node.kind {
            DynamicNodeKind::Analyze { operation_ref, .. } => Some(operation_ref.clone()),
            DynamicNodeKind::Decide { .. } => None,
        })
        .collect()
}

pub fn index_of(log: &[String], entry: &str) -> usize {
    assert!(
        log.iter().any(|line| line == entry),
        "log has {entry}: {log:?}"
    );
    log.iter()
        .position(|line| line == entry)
        .expect("entry present")
}

pub fn limits(max_parallel_analyses: u64) -> WorkflowLimits {
    WorkflowLimits {
        max_steps: 16,
        max_subworkflow_depth: 1,
        max_provider_calls: 8,
        max_parallel_analyses,
        max_output_tokens: 256,
    }
}
