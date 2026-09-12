// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};

/// Redacted provider-session evidence exposed by the workflow owner. This is
/// deliberately a metadata-only response, not a native-provider protocol.
pub const PROVIDER_SESSION_LIST_SCHEMA_VERSION: &str = "ascension.provider-session.api-result.v1";

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
