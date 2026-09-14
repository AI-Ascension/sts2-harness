// SPDX-License-Identifier: MIT

use std::error::Error;

use serde_json::{Value, json};

pub(crate) fn definition() -> Result<Value, Box<dyn Error>> {
    let mut value: Value = serde_json::from_slice(include_bytes!(
        "../../../../conformance/workflow-v1/valid-strict.json"
    ))?;
    value["annotations"]["synthetic"] = json!(false);
    value["game_profile"] = json!("sts2-live-v1");
    value["policy_ref"] = json!("policy.live.v1");
    value["graphs"][0]["nodes"][0]["config"]["projection_ref"] = json!("fair-play.live.v1");
    value["graphs"][0]["nodes"][1]["config"]["decision_profile_ref"] = json!("decision.live.v1");
    value["graphs"][0]["nodes"][1]["config"]["context_ref"] = json!("context.live.v1");
    value["capabilities"]["required"][0] = json!("observe.fair-play.live.v1");
    Ok(value)
}
