// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};

use super::ids::{
    ArtifactKindId, ContextId, DecisionProfileId, FieldId, GraphId, GuardId, LabelId, NodeId,
    OperationRef, PinnedRef, PlannerProfileId, ProjectionId, ReasonCode, RegionId, SelectorId,
};
use super::values::ValueType;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Observe,
    AwaitStability,
    Route,
    Analyze,
    Decide,
    AdaptiveRegion,
    ExecuteAction,
    Subworkflow,
    Loop,
    Checkpoint,
    EmitArtifact,
    Pause,
    Terminal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum NodeDefinition {
    #[serde(rename = "observe")]
    Observe { id: NodeId, config: ObserveConfig },
    #[serde(rename = "await_stability")]
    AwaitStability {
        id: NodeId,
        config: AwaitStabilityConfig,
    },
    #[serde(rename = "route")]
    Route { id: NodeId, config: RouteConfig },
    #[serde(rename = "analyze")]
    Analyze { id: NodeId, config: AnalysisConfig },
    #[serde(rename = "decide")]
    Decide { id: NodeId, config: DecideConfig },
    #[serde(rename = "adaptive_region")]
    AdaptiveRegion {
        id: NodeId,
        config: AdaptiveRegionConfig,
    },
    #[serde(rename = "execute_action")]
    ExecuteAction {
        id: NodeId,
        config: ExecuteActionConfig,
    },
    #[serde(rename = "subworkflow")]
    Subworkflow {
        id: NodeId,
        config: SubworkflowConfig,
    },
    #[serde(rename = "loop")]
    Loop { id: NodeId, config: LoopConfig },
    #[serde(rename = "checkpoint")]
    Checkpoint {
        id: NodeId,
        config: CheckpointConfig,
    },
    #[serde(rename = "emit_artifact")]
    EmitArtifact {
        id: NodeId,
        config: EmitArtifactConfig,
    },
    #[serde(rename = "pause")]
    Pause { id: NodeId, config: PauseConfig },
    #[serde(rename = "terminal")]
    Terminal { id: NodeId, config: TerminalConfig },
}

macro_rules! node_accessors {
    ($($variant:ident),+ $(,)?) => {
        pub fn id(&self) -> &NodeId {
            match self { $(Self::$variant { id, .. } => id,)+ }
        }

        pub fn kind(&self) -> NodeKind {
            match self { $(Self::$variant { .. } => NodeKind::$variant,)+ }
        }
    };
}

impl NodeDefinition {
    node_accessors!(
        Observe,
        AwaitStability,
        Route,
        Analyze,
        Decide,
        AdaptiveRegion,
        ExecuteAction,
        Subworkflow,
        Loop,
        Checkpoint,
        EmitArtifact,
        Pause,
        Terminal
    );

    pub fn output_type(&self, output: &str) -> Option<ValueType> {
        match self {
            Self::Observe { .. } => (output == "observation").then_some(ValueType::Observation),
            Self::AwaitStability { .. } => {
                (output == "observation").then_some(ValueType::Observation)
            }
            Self::Route { .. } => (output == "route").then_some(ValueType::Text),
            Self::Analyze { .. } => (output == "analysis").then_some(ValueType::Analysis),
            Self::Decide { .. } | Self::AdaptiveRegion { .. } => {
                (output == "proposal").then_some(ValueType::DecisionProposal)
            }
            Self::ExecuteAction { .. } => (output == "result").then_some(ValueType::Null),
            Self::Subworkflow { .. } => {
                (output == "selection").then_some(ValueType::SubworkflowSelection)
            }
            Self::Loop { .. } => (output == "result").then_some(ValueType::Unknown),
            Self::Checkpoint { .. } | Self::Pause { .. } => {
                (output == "result").then_some(ValueType::Null)
            }
            Self::EmitArtifact { .. } => (output == "artifact").then_some(ValueType::Artifact),
            Self::Terminal { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObserveConfig {
    pub projection_ref: ProjectionId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AwaitStabilityConfig {
    pub deadline_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteConfig {
    pub selector_ref: SelectorId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisConfig {
    pub operation_ref: OperationRef,
    pub context_ref: ContextId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecideConfig {
    pub decision_profile_ref: DecisionProfileId,
    pub context_ref: ContextId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdaptiveRegionConfig {
    pub region_id: RegionId,
    pub planner_profile_ref: PlannerProfileId,
    pub allowed_operations: Vec<OperationRef>,
    pub max_plan_nodes: u64,
    pub max_plan_edges: u64,
    pub max_replans: u64,
    pub output_type: AdaptiveOutputType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdaptiveOutputType {
    #[serde(rename = "DecisionProposal")]
    DecisionProposal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalBinding {
    pub node_id: NodeId,
    pub output: super::ids::OutputId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecuteActionConfig {
    pub proposal_from: ProposalBinding,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubworkflowConfig {
    pub artifact_ref: PinnedRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoopConfig {
    pub body_graph: GraphId,
    pub max_iterations: u64,
    pub exit_guard_ref: GuardId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointConfig {
    pub label: LabelId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmitArtifactConfig {
    pub artifact_kind_ref: ArtifactKindId,
    pub input_from: ProposalBinding,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PauseConfig {
    pub reason_code: ReasonCode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TerminalConfig {
    pub outcome: TerminalOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalOutcome {
    Completed,
    Failed,
    NeedsOperator,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardDefinition {
    pub id: GuardId,
    pub expression: GuardExpression,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum GuardExpression {
    Literal(GuardValue),
    Field(FieldId),
    Exists(FieldId),
    Equal {
        left: FieldId,
        right: GuardValue,
    },
    NotEqual {
        left: FieldId,
        right: GuardValue,
    },
    Less {
        left: FieldId,
        right: GuardValue,
    },
    LessOrEqual {
        left: FieldId,
        right: GuardValue,
    },
    Greater {
        left: FieldId,
        right: GuardValue,
    },
    GreaterOrEqual {
        left: FieldId,
        right: GuardValue,
    },
    And(Vec<GuardExpression>),
    Or(Vec<GuardExpression>),
    Not(Box<GuardExpression>),
    In {
        field: FieldId,
        values: Vec<GuardValue>,
    },
    Add {
        left: FieldId,
        right: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum GuardValue {
    Null,
    Boolean(bool),
    Integer(i64),
    Text(String),
}
