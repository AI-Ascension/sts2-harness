// SPDX-License-Identifier: MIT

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};

use super::definition::AdaptiveRegionConfig;
use super::ids::{
    ContextId, Digest, NodeId, OperationRef, PlanId, PlannerProfileId, RegionId, Revision,
};
use super::values::AnalysisValue;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum DynamicNodeKind {
    #[serde(rename = "analyze")]
    Analyze {
        operation_ref: OperationRef,
        context_ref: ContextId,
    },
    #[serde(rename = "decide")]
    Decide {
        decision_profile_ref: super::ids::DecisionProfileId,
        context_ref: ContextId,
        inputs: Vec<NodeId>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DynamicNode {
    pub id: NodeId,
    pub kind: DynamicNodeKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DynamicEdge {
    pub from: NodeId,
    pub to: NodeId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DynamicPlan {
    pub plan_id: PlanId,
    pub region_id: RegionId,
    pub planner_profile_ref: PlannerProfileId,
    pub base_semantic_digest: Digest,
    pub base_revision: Revision,
    pub dependency_digest: Digest,
    pub nodes: Vec<DynamicNode>,
    pub edges: Vec<DynamicEdge>,
}

impl DynamicPlan {
    pub fn digest(&self) -> Result<Digest, DynamicPlanError> {
        let bytes = serde_json::to_vec(self).map_err(|_| DynamicPlanError::InvalidPlan)?;
        Ok(Digest::sha256(&bytes))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DynamicPlanError {
    InvalidPlan,
    UnknownOperation,
    DuplicateIdentifier,
    MissingReference,
    Cycle,
    Capacity,
    BaseRevisionChanged,
    DependencyChanged,
    ReplanLimit,
    NoProgress,
    Persistence,
}

impl std::fmt::Display for DynamicPlanError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidPlan => "dynamic plan is invalid",
            Self::UnknownOperation => "dynamic plan contains an unadmitted operation",
            Self::DuplicateIdentifier => "dynamic plan contains a duplicate identifier",
            Self::MissingReference => "dynamic plan contains a missing reference",
            Self::Cycle => "dynamic plan contains a cycle",
            Self::Capacity => "dynamic plan exceeds its declared capacity",
            Self::BaseRevisionChanged => "dynamic plan base revision is stale",
            Self::DependencyChanged => "dynamic plan dependencies are stale",
            Self::ReplanLimit => "dynamic plan replan budget is exhausted",
            Self::NoProgress => "dynamic plan made no progress",
            Self::Persistence => "dynamic plan could not be persisted before execution",
        })
    }
}

impl std::error::Error for DynamicPlanError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DynamicPlanResult {
    pub plan_digest: Digest,
    pub analyses: BTreeMap<NodeId, AnalysisValue>,
}

pub trait PlanStore {
    fn persist_plan(&mut self, plan: &DynamicPlan, digest: &Digest)
    -> Result<(), DynamicPlanError>;
}

pub trait PureAnalysisExecutor {
    fn analyze(
        &mut self,
        operation: &OperationRef,
        context: &ContextId,
        inputs: &BTreeMap<NodeId, AnalysisValue>,
    ) -> Result<AnalysisValue, DynamicPlanError>;
}

pub struct DynamicPlanRegistry {
    region_id: RegionId,
    planner_profile_ref: PlannerProfileId,
    base_semantic_digest: Digest,
    base_revision: Revision,
    allowed_operations: BTreeSet<OperationRef>,
    max_nodes: usize,
    max_edges: usize,
    max_replans: u64,
    replans: u64,
    last_plan_digest: Option<Digest>,
    last_dependency_digest: Option<Digest>,
}

impl DynamicPlanRegistry {
    pub fn from_region(
        region: &AdaptiveRegionConfig,
        base_semantic_digest: Digest,
        base_revision: Revision,
    ) -> Result<Self, DynamicPlanError> {
        if region.allowed_operations.is_empty()
            || region.max_plan_nodes == 0
            || region.max_plan_edges == 0
        {
            return Err(DynamicPlanError::InvalidPlan);
        }
        Ok(Self {
            region_id: region.region_id.clone(),
            planner_profile_ref: region.planner_profile_ref.clone(),
            base_semantic_digest,
            base_revision,
            allowed_operations: region.allowed_operations.iter().cloned().collect(),
            max_nodes: usize::try_from(region.max_plan_nodes)
                .map_err(|_| DynamicPlanError::Capacity)?,
            max_edges: usize::try_from(region.max_plan_edges)
                .map_err(|_| DynamicPlanError::Capacity)?,
            max_replans: region.max_replans,
            replans: 0,
            last_plan_digest: None,
            last_dependency_digest: None,
        })
    }

    pub fn accept<S: PlanStore>(
        &mut self,
        plan: DynamicPlan,
        store: &mut S,
    ) -> Result<Digest, DynamicPlanError> {
        if self.replans >= self.max_replans {
            return Err(DynamicPlanError::ReplanLimit);
        }
        if plan.region_id != self.region_id
            || plan.planner_profile_ref != self.planner_profile_ref
            || plan.base_semantic_digest != self.base_semantic_digest
            || plan.base_revision != self.base_revision
        {
            return Err(DynamicPlanError::BaseRevisionChanged);
        }
        if self
            .last_dependency_digest
            .as_ref()
            .is_some_and(|digest| digest != &plan.dependency_digest)
        {
            return Err(DynamicPlanError::DependencyChanged);
        }
        validate_plan(
            &plan,
            &self.allowed_operations,
            self.max_nodes,
            self.max_edges,
        )?;
        let digest = plan.digest()?;
        if self.last_plan_digest.as_ref() == Some(&digest) {
            return Err(DynamicPlanError::NoProgress);
        }
        store.persist_plan(&plan, &digest)?;
        self.replans = self.replans.saturating_add(1);
        self.last_plan_digest = Some(digest.clone());
        self.last_dependency_digest = Some(plan.dependency_digest);
        Ok(digest)
    }

    pub fn invalidate_for_dependencies(&mut self, dependency_digest: &Digest) -> bool {
        if self
            .last_dependency_digest
            .as_ref()
            .is_some_and(|current| current != dependency_digest)
        {
            self.last_plan_digest = None;
            self.last_dependency_digest = None;
            true
        } else {
            false
        }
    }

    #[must_use]
    pub const fn replans(&self) -> u64 {
        self.replans
    }
}

pub fn validate_plan(
    plan: &DynamicPlan,
    allowed_operations: &BTreeSet<OperationRef>,
    max_nodes: usize,
    max_edges: usize,
) -> Result<Vec<NodeId>, DynamicPlanError> {
    if plan.nodes.is_empty()
        || plan.nodes.len() > max_nodes
        || plan.edges.len() > max_edges
        || plan.nodes.iter().any(|node| match &node.kind {
            DynamicNodeKind::Analyze { operation_ref, .. } => {
                !allowed_operations.contains(operation_ref)
            }
            DynamicNodeKind::Decide { .. } => false,
        })
    {
        return Err(
            if plan.nodes.len() > max_nodes || plan.edges.len() > max_edges {
                DynamicPlanError::Capacity
            } else {
                DynamicPlanError::UnknownOperation
            },
        );
    }
    let mut nodes = BTreeSet::new();
    for node in &plan.nodes {
        if !nodes.insert(node.id.clone()) {
            return Err(DynamicPlanError::DuplicateIdentifier);
        }
        if let DynamicNodeKind::Decide { inputs, .. } = &node.kind
            && inputs.iter().any(|input| !nodes.contains(input))
        {
            return Err(DynamicPlanError::MissingReference);
        }
    }
    let mut adjacency: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
    let mut incoming: BTreeMap<NodeId, usize> = nodes.iter().cloned().map(|id| (id, 0)).collect();
    let mut edge_keys = BTreeSet::new();
    for edge in &plan.edges {
        if !nodes.contains(&edge.from)
            || !nodes.contains(&edge.to)
            || !edge_keys.insert((edge.from.clone(), edge.to.clone()))
        {
            return Err(DynamicPlanError::MissingReference);
        }
        adjacency
            .entry(edge.from.clone())
            .or_default()
            .push(edge.to.clone());
        if let Some(count) = incoming.get_mut(&edge.to) {
            *count = count.saturating_add(1);
        }
    }
    let mut queue: VecDeque<NodeId> = incoming
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(id, _)| id.clone())
        .collect();
    let mut order = Vec::with_capacity(nodes.len());
    while let Some(node) = queue.pop_front() {
        order.push(node.clone());
        for child in adjacency.get(&node).into_iter().flatten() {
            if let Some(count) = incoming.get_mut(child) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    queue.push_back(child.clone());
                }
            }
        }
    }
    if order.len() != nodes.len() {
        return Err(DynamicPlanError::Cycle);
    }
    Ok(order)
}

pub fn execute_plan<A: PureAnalysisExecutor>(
    plan: &DynamicPlan,
    order: &[NodeId],
    analysis: &mut A,
) -> Result<DynamicPlanResult, DynamicPlanError> {
    let by_id: BTreeMap<NodeId, &DynamicNode> = plan
        .nodes
        .iter()
        .map(|node| (node.id.clone(), node))
        .collect();
    let mut values = BTreeMap::new();
    for id in order {
        let node = by_id.get(id).ok_or(DynamicPlanError::MissingReference)?;
        match &node.kind {
            DynamicNodeKind::Analyze {
                operation_ref,
                context_ref,
            } => {
                let value = analysis.analyze(operation_ref, context_ref, &values)?;
                values.insert(id.clone(), value);
            }
            DynamicNodeKind::Decide { inputs, .. } => {
                if inputs.iter().any(|input| !values.contains_key(input)) {
                    return Err(DynamicPlanError::MissingReference);
                }
                let value = analysis.analyze(
                    &OperationRef::new("decision.compose")
                        .map_err(|_| DynamicPlanError::InvalidPlan)?,
                    &ContextId::new("decision").map_err(|_| DynamicPlanError::InvalidPlan)?,
                    &values,
                )?;
                values.insert(id.clone(), value);
            }
        }
    }
    Ok(DynamicPlanResult {
        plan_digest: plan.digest()?,
        analyses: values,
    })
}
