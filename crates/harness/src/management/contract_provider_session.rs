// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};

/// Redacted provider-session evidence exposed by the workflow owner. This is
/// deliberately a metadata-only response, not a native-provider protocol.
pub const PROVIDER_SESSION_LIST_SCHEMA_VERSION: &str = "ascension.provider-session.api-result.v1";
pub const PROVIDER_SESSION_POLICY_VIEW_SCHEMA_VERSION: &str =
    "ascension.provider-session.policy-owner-view.v1";
pub const PROVIDER_SESSION_POLICY_COMMAND_SCHEMA_VERSION: &str =
    "ascension.provider-session.policy-owner-command.v1";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderSessionPolicyApprovalRequest {
    pub schema_version: String,
    pub proposal_sha256: String,
    pub approval_ref: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderSessionPolicyAdoptImportedRequest {
    pub schema_version: String,
    pub policy_sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderSessionPolicyCommandResponse {
    pub schema_version: String,
    pub operation: String,
    pub revision: u64,
    pub policy_sha256: Option<String>,
    pub proposal_sha256: Option<String>,
    pub effect_class: String,
    pub inference_calls: u64,
    pub game_effects: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderSessionPolicyViewResponse {
    pub schema_version: String,
    pub operation: String,
    pub value: ProviderSessionPolicyViewValue,
    pub effect_class: String,
    pub inference_calls: u64,
    pub game_effects: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderSessionPolicyViewValue {
    pub run_id: String,
    pub revision: u64,
    pub active: Option<ProviderSessionPolicyBindingMetadata>,
    pub history: Vec<ProviderSessionPolicyHistoryMetadata>,
    pub proposals: Vec<ProviderSessionPolicyProposalMetadata>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderSessionPolicyBindingMetadata {
    pub sha256: String,
    pub policy_id: String,
    pub version: u64,
    pub mode: String,
    pub continuity: String,
    pub max_completed_turns: usize,
    pub history_ttl_seconds: u64,
    pub epoch: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderSessionPolicyHistoryMetadata {
    pub sha256: String,
    pub policy_id: String,
    pub version: u64,
    pub mode: String,
    pub continuity: String,
    pub active: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderSessionPolicyProposalMetadata {
    pub proposal_id: String,
    pub source_sha256: String,
    pub target_sha256: String,
    pub state: String,
    pub approval_recorded: bool,
    pub adopted_policy_sha256: Option<String>,
}

/// A bounded, redacted provider-session projection. The `run_id` is always
/// the workflow-management run identity; an adapter must not infer it from a
/// context or native-provider identifier.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderSessionListResponse {
    pub schema: String,
    pub operation: String,
    pub value: ProviderSessionListValue,
    pub effect_class: String,
    pub inference_calls: u64,
    pub game_effects: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderSessionListValue {
    pub run_id: String,
    pub bindings: Vec<ProviderSessionBindingSummary>,
    pub operations: Vec<ProviderSessionOperationSummary>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderSessionBindingSummary {
    pub binding_id: String,
    pub state: String,
    pub history_coverage: String,
    pub game_dispatch_capability: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderSessionOperationSummary {
    pub operation_id: String,
    pub state: String,
    pub game_effects: u64,
    pub auto_resume: bool,
}
