// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::definition::{EdgeOutcome, GuardValue, NodeDefinition, TerminalOutcome};
use super::ids::{Digest, FieldId, GraphId, NodeId};
use super::values::TypedValue;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeStatus {
    Running,
    Paused,
    Completed,
    Failed,
    NeedsOperator,
}

impl RuntimeStatus {
    #[must_use]
    pub const fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::NeedsOperator)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeFault {
    InvalidState,
    BudgetExceeded,
    MissingRoute,
    GuardFailure,
    TypeMismatch,
    UnknownEffect,
    ExecutorUnavailable,
    ExecutorRejected,
    PlanRejected,
}

impl std::fmt::Display for RuntimeFault {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidState => "workflow runtime state is invalid",
            Self::BudgetExceeded => "workflow runtime budget is exhausted",
            Self::MissingRoute => "workflow node has no eligible route",
            Self::GuardFailure => "workflow guard evaluation failed",
            Self::TypeMismatch => "workflow node produced an unexpected value type",
            Self::UnknownEffect => "workflow mutation outcome is unresolved",
            Self::ExecutorUnavailable => "workflow node executor is unavailable",
            Self::ExecutorRejected => "workflow node executor rejected the operation",
            Self::PlanRejected => "workflow plan was rejected",
        })
    }
}

impl std::error::Error for RuntimeFault {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeOutcome {
    pub outcome: EdgeOutcome,
    pub value: TypedValue,
    pub guard_fields: BTreeMap<FieldId, GuardValue>,
}

impl NodeOutcome {
    #[must_use]
    pub fn new(outcome: EdgeOutcome, value: TypedValue) -> Self {
        Self {
            outcome,
            value,
            guard_fields: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn with_guard_field(mut self, field: FieldId, value: GuardValue) -> Self {
        self.guard_fields.insert(field, value);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeContext {
    pub graph_id: GraphId,
    pub node_id: NodeId,
    pub step: u64,
    pub outputs: BTreeMap<String, TypedValue>,
    pub guard_fields: BTreeMap<FieldId, GuardValue>,
}

pub trait NodeExecutor {
    fn execute(
        &mut self,
        node: &NodeDefinition,
        context: &RuntimeContext,
    ) -> Result<NodeOutcome, RuntimeFault>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReturnFrame {
    pub graph_id: GraphId,
    pub node_id: NodeId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeSnapshot {
    pub semantic_digest: Digest,
    pub graph_id: GraphId,
    pub node_id: NodeId,
    pub stack: Vec<ReturnFrame>,
    pub steps: u64,
    pub status: RuntimeStatus,
    pub outputs: BTreeMap<String, TypedValue>,
    pub guard_fields: BTreeMap<FieldId, GuardValue>,
    pub loop_iterations: BTreeMap<String, u64>,
    pub event_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeEvent {
    pub sequence: u64,
    pub graph_id: GraphId,
    pub node_id: NodeId,
    pub outcome: Option<EdgeOutcome>,
    pub status: RuntimeStatus,
    pub terminal: Option<TerminalOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeRunReport {
    pub status: RuntimeStatus,
    pub terminal: Option<TerminalOutcome>,
    pub steps: u64,
    pub events: Vec<RuntimeEvent>,
    pub snapshot: RuntimeSnapshot,
}
