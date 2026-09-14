// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use sts2_harness::EXO_SOURCE_REVISION;

const EXPERT_OBSERVATION: &[u8] =
    include_bytes!("../../../../protocol-artifact/runtime-v4-expert/golden/observation.json");

pub(super) fn expert_request_variant(mutation: &str) -> Value {
    let mut request = expert_request_value();
    match mutation {
        "none" => {}
        "schema_digest_zero" => request["observation"]["schema_digest"] = json!("0".repeat(64)),
        "empty_legal_actions" => request["observation"]["legal_actions"] = json!([]),
        "player_hp_above_max" => request["observation"]["player"]["hp"] = json!(81),
        "duplicate_action_id" => {
            let first_id = request["observation"]["legal_actions"][0]["action_id"].clone();
            request["observation"]["legal_actions"][1]["action_id"] = first_id;
        }
        "multibyte_text_over_utf8_bound" => {
            request["observation"]["player"]["hand"][0]["name"] = json!("é".repeat(257));
        }
        other => unreachable!("unhandled expert request mutation {other}"),
    }
    request
}

fn expert_request_value() -> Value {
    let observation: Value =
        serde_json::from_slice(EXPERT_OBSERVATION).expect("expert observation fixture is JSON");
    json!({
        "schema": "sts2.exo-decision-v1",
        "provider_revision": EXO_SOURCE_REVISION,
        "model_execution_id": "execution-expert",
        "state_id": "live:7",
        "generation": 7,
        "observation": observation,
        "legal_action_ids": [
            "play:7:card:1:enemy:1",
            "potion:7:potion:fire:enemy:1",
            "end:7"
        ],
        "objective": "decide",
        "hard_constraints": [],
        "max_response_bytes": 8192
    })
}
