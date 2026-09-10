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
include!("runtime_v4_expert_fair_play.rs");

#[cfg(test)]
mod fair_play_tests {
    use super::*;

    #[test]
    fn projected_selector_forms_use_a_separate_provider_validation_path()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut value: Value = serde_json::from_str(include_str!(
            "../../../protocol-artifact/runtime-v4-expert/golden/observation.json"
        ))?;
        value["legal_actions"] = serde_json::json!([
            {"action_id":"select-card:7:card:1", "action":{"kind":"select_card", "card_id":"card:1"}},
            {"action_id":"confirm:7", "action":{"kind":"confirm_selection"}},
            {"action_id":"select-player:7:player:local", "action":{"kind":"select_player", "player_id":"player:local"}}
        ]);
        assert_eq!(
            RuntimeV4ExpertObservation::from_value(value.clone()),
            Err(RuntimeV4ExpertParseError::InvalidShape)
        );
        assert_eq!(
            crate::SanitizedObservation::new(value.clone()),
            Err(crate::SandboxError::InvalidExpertObservation)
        );
        value["harness_projection"] =
            Value::String(crate::RUNTIME_V4_EXPERT_FAIR_PLAY_PROJECTION.to_owned());
        assert!(crate::SanitizedObservation::new(value).is_ok());
        Ok(())
    }
}
