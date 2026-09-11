// SPDX-License-Identifier: MIT

use serde::{
    Deserialize, Deserializer,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Value};

use crate::runtime_v4_expert_action_artifact::{
    RUNTIME_V4_EXPERT_ACTION_ARTIFACT, RUNTIME_V4_EXPERT_ACTION_GENERATOR,
    RUNTIME_V4_EXPERT_ACTION_PROTOCOL_VERSION, RUNTIME_V4_EXPERT_ACTION_SCHEMA_DIGEST,
    RUNTIME_V4_EXPERT_ACTION_SCHEMA_SOURCE, verify_runtime_v4_expert_action_artifact,
};

const MAX_ACTION_BYTES: usize = 128 * 1024;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const MAX_IDENTITY_BYTES: usize = 512;

include!("runtime_v4_expert_action_types.rs");
include!("runtime_v4_expert_action_validation.rs");
include!("runtime_v4_expert_action_strict.rs");
