// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};

use super::ids::{ActionId, BoundedText, Digest, Generation, PinnedRef, ProviderExecutionId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionProposal {
    pub state_id: BoundedText,
    pub generation: Generation,
    pub catalog_digest: Digest,
    pub action_id: ActionId,
    pub provider_execution_id: ProviderExecutionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<BoundedText>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubworkflowSelection {
    pub artifact_ref: PinnedRef,
    pub input_digest: Digest,
}
