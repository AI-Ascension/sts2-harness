// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::collections::{BTreeMap, BTreeSet};

use sts2_harness::workflow::{
    AdaptiveRegionConfig, AnalysisValue, BoundedText, ContextId, Digest, DynamicEdge, DynamicNode,
    DynamicNodeKind, DynamicPlan, DynamicPlanError, DynamicPlanRegistry, NodeId, OperationRef,
    PlanId, PlanStore, PlannerProfileId, PureAnalysisExecutor, RegionId, Revision, execute_plan,
    validate_plan,
};

fn plan(dependency_digest: Digest) -> DynamicPlan {
    DynamicPlan {
        plan_id: PlanId::new("plan-1").expect("plan id"),
        region_id: RegionId::new("region-1").expect("region id"),
        planner_profile_ref: PlannerProfileId::new("planner-1").expect("planner id"),
        base_semantic_digest: Digest::sha256(b"workflow"),
        base_revision: Revision::new(1).expect("revision"),
        dependency_digest,
        nodes: vec![DynamicNode {
            id: NodeId::new("analysis").expect("node id"),
            kind: DynamicNodeKind::Analyze {
                operation_ref: OperationRef::new("map.inspect").expect("operation"),
                context_ref: ContextId::new("context-1").expect("context"),
            },
        }],
        edges: Vec::new(),
    }
}

fn region() -> AdaptiveRegionConfig {
    AdaptiveRegionConfig {
        region_id: RegionId::new("region-1").expect("region id"),
        planner_profile_ref: PlannerProfileId::new("planner-1").expect("planner id"),
        allowed_operations: vec![OperationRef::new("map.inspect").expect("operation")],
        max_plan_nodes: 4,
        max_plan_edges: 4,
        max_replans: 2,
        output_type: sts2_harness::workflow::AdaptiveOutputType::DecisionProposal,
    }
}

#[derive(Default)]
struct MemoryStore {
    persisted: Vec<Digest>,
}

impl PlanStore for MemoryStore {
    fn persist_plan(
        &mut self,
        _plan: &DynamicPlan,
        digest: &Digest,
    ) -> Result<(), DynamicPlanError> {
        self.persisted.push(digest.clone());
        Ok(())
    }
}

struct Analyzer;

impl PureAnalysisExecutor for Analyzer {
    fn analyze(
        &mut self,
        _operation: &OperationRef,
        _context: &ContextId,
        _inputs: &BTreeMap<NodeId, AnalysisValue>,
    ) -> Result<AnalysisValue, DynamicPlanError> {
        Ok(AnalysisValue {
            code: BoundedText::new("ready").expect("code"),
            fields: BTreeMap::new(),
        })
    }
}

#[test]
fn plan_is_persisted_before_pure_analysis_and_duplicate_progress_is_rejected() {
    let mut registry = DynamicPlanRegistry::from_region(
        &region(),
        Digest::sha256(b"workflow"),
        Revision::new(1).expect("revision"),
    )
    .expect("registry");
    let mut store = MemoryStore::default();
    let plan = plan(Digest::sha256(b"inputs"));
    let digest = registry
        .accept(plan.clone(), &mut store)
        .expect("plan accepted");
    assert_eq!(store.persisted, vec![digest]);
    assert_eq!(
        registry.accept(plan, &mut store),
        Err(DynamicPlanError::NoProgress)
    );
}

#[test]
fn dependency_changes_invalidate_the_accepted_plan() {
    let mut registry = DynamicPlanRegistry::from_region(
        &region(),
        Digest::sha256(b"workflow"),
        Revision::new(1).expect("revision"),
    )
    .expect("registry");
    let mut store = MemoryStore::default();
    registry
        .accept(plan(Digest::sha256(b"inputs")), &mut store)
        .expect("plan accepted");
    assert!(registry.invalidate_for_dependencies(&Digest::sha256(b"changed")));
    assert_eq!(registry.replans(), 1);
}

#[test]
fn typed_dag_validation_and_deterministic_execution_are_bounded() {
    let plan = plan(Digest::sha256(b"inputs"));
    let allowed = BTreeSet::from([OperationRef::new("map.inspect").expect("operation")]);
    let order = validate_plan(&plan, &allowed, 4, 4).expect("plan validates");
    let result = execute_plan(&plan, &order, &mut Analyzer).expect("plan executes");
    assert_eq!(result.analyses.len(), 1);
    assert_eq!(result.plan_digest, plan.digest().expect("digest"));
}

#[test]
fn cycles_and_unadmitted_operations_fail_before_execution() {
    let mut plan = plan(Digest::sha256(b"inputs"));
    plan.nodes.push(DynamicNode {
        id: NodeId::new("second").expect("node id"),
        kind: DynamicNodeKind::Analyze {
            operation_ref: OperationRef::new("secret.write").expect("operation"),
            context_ref: ContextId::new("context-1").expect("context"),
        },
    });
    plan.edges = vec![
        DynamicEdge {
            from: NodeId::new("analysis").expect("node id"),
            to: NodeId::new("second").expect("node id"),
        },
        DynamicEdge {
            from: NodeId::new("second").expect("node id"),
            to: NodeId::new("analysis").expect("node id"),
        },
    ];
    let allowed = BTreeSet::from([OperationRef::new("map.inspect").expect("operation")]);
    assert_eq!(
        validate_plan(&plan, &allowed, 4, 4),
        Err(DynamicPlanError::UnknownOperation)
    );
}
