// SPDX-License-Identifier: MIT

use std::collections::BTreeSet;

use serde::{
    Deserialize, Deserializer,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Value};

use crate::runtime_v4_expert_artifact::{
    RUNTIME_V4_EXPERT_ARTIFACT, RUNTIME_V4_EXPERT_GENERATOR, RUNTIME_V4_EXPERT_PROTOCOL_VERSION,
    RUNTIME_V4_EXPERT_SCHEMA_DIGEST, RUNTIME_V4_EXPERT_SCHEMA_SOURCE,
    verify_runtime_v4_expert_artifact,
};

const MAX_OBSERVATION_BYTES: usize = 128 * 1024;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const MAX_TEXT_BYTES: usize = 512;
const MAX_ITEMS: usize = 256;
const MAX_EDGES: usize = 512;
const MAX_TARGETS: usize = 16;

include!("runtime_v4_expert_parse.rs");
include!("runtime_v4_expert_types_a.rs");
include!("runtime_v4_expert_types_b.rs");
include!("runtime_v4_expert_validation.rs");
include!("runtime_v4_expert_shape_root.rs");
include!("runtime_v4_expert_shape_collections.rs");
include!("runtime_v4_expert_shape_actions.rs");
