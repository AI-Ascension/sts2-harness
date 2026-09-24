// SPDX-License-Identifier: MIT

mod artifact;
mod bounded_region;
mod canonical;
mod compiler;
mod decoder;
mod definition;
mod diagnostic;
mod dynamic;
mod dynamic_budget;
mod dynamic_join;
mod dynamic_parallel;
mod dynamic_runtime;
mod graph_checks;
mod graph_validation;
mod guards;
mod id_types;
mod ids;
mod node;
mod plans;
mod provider;
mod runtime;
mod runtime_types;
mod scanner;
mod validation;
mod values;

pub mod types {
    pub use super::ids::*;
    pub use super::plans::*;
    pub use super::values::*;
}

pub use artifact::{
    ArtifactError, ArtifactManifest, CatalogError, CatalogInsert, WorkflowArtifact, WorkflowCatalog,
};
pub use bounded_region::{
    BOUNDED_ANALYSIS_REPORT_SCHEMA, BoundedAnalysisOutcome, BoundedBranchState,
    BoundedRegionRefusal, admit_bounded_region, admitted_nodes, is_analysis_kind, reason_of,
    run_bounded_region,
};
pub use canonical::{CanonicalError, SemanticDiff, canonical_json_bytes, semantic_diff};
pub use compiler::{CompileError, CompiledGraph, CompiledWorkflow};
pub use decoder::{
    DecodeError, DecoderLimits, MAX_COLLECTION_ITEMS, MAX_DEPTH, MAX_SOURCE_BYTES, decode_json,
    decode_strict, decode_strict_with_limits,
};
pub use definition::{
    AdaptiveOutputType, AdaptiveRegionConfig, AnalysisConfig, Annotations, AwaitStabilityConfig,
    BindingSource, CheckpointConfig, ControlEdge, DataBinding, DecideConfig, EdgeOutcome,
    EmitArtifactConfig, ExecuteActionConfig, GraphDefinition, GuardDefinition, GuardExpression,
    GuardValue, LoopConfig, NodeDefinition, NodeKind, ObserveConfig, PauseConfig, ProposalBinding,
    RouteConfig, SubworkflowConfig, TerminalConfig, TerminalOutcome, WORKFLOW_COMPILER_ID,
    WorkflowCapabilities, WorkflowDefinition, WorkflowLimits, WorkflowMode, WorkflowSchemaVersion,
};
pub use diagnostic::{
    Diagnostic, DiagnosticCode, DiagnosticReport, DiagnosticSeverity, StructuralLocation,
};
pub use dynamic::{
    AnalysisFault, DynamicEdge, DynamicNode, DynamicNodeKind, DynamicPlan, DynamicPlanError,
    DynamicPlanRegistry, DynamicPlanResult, ParallelAnalysisExecutor, PlanStore,
    PureAnalysisExecutor, execute_plan, validate_plan,
};
pub use dynamic_budget::{
    BranchBudgetKey, BranchBudgetLedger, BranchReservation, CancelFlag, CancelSignal,
    MAX_BUDGET_UNITS, ParallelBudget, execute_plan_bounded_reserved,
};
pub use dynamic_join::{BranchOutcome, JoinedResult, MAX_PARALLEL_ANALYSES, ParallelCap};
pub use dynamic_parallel::execute_plan_bounded;
pub use dynamic_runtime::{DynamicExecutorPort, DynamicRuntime};
pub use guards::{GuardContext, GuardError, TruthValue};
pub use ids::*;
pub use plans::{DecisionProposal, SubworkflowSelection};
pub use provider::{
    BudgetError, BudgetLedger, BudgetReservation, ProviderInput, ProviderInputError,
    ProviderProfile, ProviderRegistry, ProviderRegistryError, ReservationState,
};
pub use runtime::StrictRuntime;
pub use runtime_types::{
    NodeExecutor, NodeOutcome, ReturnFrame, RuntimeContext, RuntimeEvent, RuntimeFault,
    RuntimeRunReport, RuntimeSnapshot, RuntimeStatus,
};
pub use validation::{ValidationError, validate_definition};
pub use values::{
    AnalysisValue, ArtifactValue, ObservationValue, ScalarValue, TypedValue, ValueType,
};
