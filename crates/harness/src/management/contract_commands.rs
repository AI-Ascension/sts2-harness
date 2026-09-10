// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{RunEvent, RunSnapshot, WorkflowRunStatus};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandKind {
    Pause,
    Resume,
    Step,
    Cancel,
}

impl CommandKind {
    pub fn reason_code(&self) -> &'static str {
        match self {
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::Step => "step",
            Self::Cancel => "cancel",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CommandParameters {}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CommandRequest {
    pub schema_version: String,
    pub command_id: String,
    pub run_id: String,
    pub expected_revision: u64,
    pub actor_scope: String,
    pub kind: CommandKind,
    pub parameters: CommandParameters,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandOutcome {
    Accepted,
    Applied,
    Pending,
    Duplicate,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CommandResponse {
    pub schema_version: String,
    pub command_id: String,
    pub workflow_run_id: String,
    pub outcome: CommandOutcome,
    pub run_revision: u64,
    pub sequence: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ValidateRequest {
    pub schema_version: String,
    pub definition: Value,
    pub capabilities: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InspectRequest {
    pub schema_version: String,
    pub definition: Value,
    pub format: OutputFormat,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    Json,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DiffRequest {
    pub schema_version: String,
    pub old_definition: Value,
    pub new_definition: Value,
    pub format: OutputFormat,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RunRequest {
    pub schema_version: String,
    pub request_id: String,
    pub definition: Option<Value>,
    pub artifact_id: Option<String>,
    pub instance_id: String,
    pub profile: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReplayRequest {
    pub schema_version: String,
    pub run_id: String,
    pub offline: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExportRequest {
    pub schema_version: String,
    pub run_id: String,
    pub redacted: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ValidateResponse {
    pub schema_version: String,
    pub valid: bool,
    pub definition_digest: String,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Diagnostic {
    pub code: String,
    pub severity: DiagnosticSeverity,
    pub path: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InspectResponse {
    pub schema_version: String,
    pub definition_digest: String,
    pub workflow_id: Option<String>,
    pub workflow_version: Option<String>,
    pub required_capabilities: Vec<String>,
    pub graph_count: u64,
    pub node_count: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DiffResponse {
    pub schema_version: String,
    pub old_definition_digest: String,
    pub new_definition_digest: String,
    pub semantic_change: bool,
    pub changed_paths: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RunSubmissionResponse {
    pub schema_version: String,
    pub workflow_run_id: String,
    pub run_revision: u64,
    pub status: WorkflowRunStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AuthoritySummary {
    pub state: String,
    pub recovery: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StatusResponse {
    pub schema_version: String,
    pub run: RunSnapshot,
    pub accepted_plan_revision: Option<u64>,
    pub waiting_reason: Option<String>,
    pub authority: AuthoritySummary,
    pub last_progress_sequence: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReplayResponse {
    pub schema_version: String,
    pub workflow_run_id: String,
    pub matched: bool,
    pub compared_events: u64,
    pub first_divergence: Option<ReplayDivergence>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReplayDivergence {
    pub path: String,
    pub code: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExportResponse {
    pub schema_version: String,
    pub workflow_run_id: String,
    pub redacted: bool,
    pub run: RunSnapshot,
    pub events: Vec<RunEvent>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CapabilityResponse {
    pub schema_version: String,
    pub capabilities: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HealthResponse {
    pub schema_version: String,
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ErrorBody {
    pub class: ErrorClass,
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ErrorResponse {
    pub schema_version: String,
    pub error: ErrorBody,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorClass {
    InvalidInput,
    Capability,
    Conflict,
    Forbidden,
    Unresolved,
    Unavailable,
    Budget,
    Store,
    Replay,
    Authentication,
}
