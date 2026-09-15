// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Document {
    pub version: String,
    pub gameplay: Gameplay,
    pub experiment: Experiment,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Gameplay {
    pub requested_seed: String,
    pub effective_seed: String,
    pub seed_contract: String,
    pub selected_context: SelectedContext,
    pub game_version: String,
    pub assembly_hashes: BTreeMap<String, String>,
    pub components: Components,
    pub protocol_version: String,
    pub protocol_digest: String,
    pub coverage_version: String,
    pub coverage_digest: String,
    pub platform: Platform,
    pub profile_artifact: ProfileArtifact,
    pub unlock_progress_digest: String,
    pub gameplay_settings_digest: String,
}

// Field order is the accepted seed_transport ContextDigestInput order. This is a
// consumer of that contract, not a new context normalization or game authority.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SelectedContext {
    pub context_id: String,
    pub game_mode: String,
    pub character: String,
    pub ascension: u8,
    pub modifiers: Vec<String>,
    pub acts: Vec<String>,
    pub selection_policy: String,
    pub profile_baseline: ProfileBaseline,
    pub save_policy: String,
    pub compatibility: Compatibility,
    pub context_digest: String,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProfileBaseline {
    pub kind: String,
    pub identity: String,
    pub digest: String,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Compatibility {
    pub game: IdentityDigest,
    #[serde(rename = "mod")]
    pub mod_identity: IdentityDigest,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct IdentityDigest {
    pub identity: String,
    pub digest: String,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Components {
    pub harness: Component,
    pub mcp: Component,
    pub gateway: Component,
    pub game_mod: Component,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Component {
    pub revision: String,
    pub package_digest: String,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Platform {
    pub os: String,
    pub architecture: String,
    pub runtime_version: String,
    pub compatibility_contract: String,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProfileArtifact {
    pub reference: String,
    pub baseline_digest: String,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Experiment {
    pub provider: String,
    pub model: String,
    pub provider_revision: Availability,
    pub model_revision: Availability,
    pub prompt_digest: String,
    pub workflow_digest: String,
    pub context_digest: String,
    pub tool_policy_digest: String,
    pub inference_parameters: BTreeMap<String, String>,
    pub inference_seed: InferenceSeed,
    pub budgets: Budgets,
    pub evaluator_revision: String,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Availability {
    Known { value: String },
    Unavailable,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum InferenceSeed {
    NotRequested,
    Unavailable,
    Requested {
        value: String,
        guarantee: SamplingGuarantee,
    },
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum SamplingGuarantee {
    Unavailable,
    BestEffort,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Budgets {
    pub max_decisions: u64,
    pub max_tokens: u64,
    pub max_duration_ms: u64,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Occurrence {
    pub process_id: String,
    pub run_id: String,
    pub episode_id: String,
    pub instance_id: String,
    pub gateway_session_id: String,
    pub mcp_session_id: String,
    pub lease_id: String,
    pub lease_epoch: u64,
    pub operation_id: String,
    pub request_generation: u64,
    pub plan_digest: String,
    pub entry_ordinal: u64,
    pub run_mode: String,
    pub created_at_unix_ms: u64,
}
