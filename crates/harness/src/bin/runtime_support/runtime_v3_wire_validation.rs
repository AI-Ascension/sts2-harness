// SPDX-License-Identifier: MIT

fn has_gameplay_envelope(response: &Value) -> bool {
    response["result"]["content"][0]["text"]
        .as_str()
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .is_some_and(|value| value["protocol_version"] == "runtime-v3-gameplay")
}

fn has_expert_action_envelope(response: &Value) -> bool {
    response["result"]["content"][0]["text"]
        .as_str()
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .is_some_and(|value| value["protocol_version"] == "runtime-v4-expert-action")
}

fn has_expert_rest_action_envelope(response: &Value) -> bool {
    response["result"]["content"][0]["text"]
        .as_str()
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .is_some_and(|value| value["protocol_version"] == "runtime-v4-expert-rest-action-v1")
}

fn has_receipt_query_envelope(response: &Value) -> bool {
    response["result"]["content"][0]["text"]
        .as_str()
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .is_some_and(|value| value["protocol_version"] == "coop-receipt-query-v1")
}

fn request_timeout(method: &str, params: &Value) -> Result<std::time::Duration, String> {
    let wait = if method == "tools/call" && params["name"] == "sts2.wait_for_transition" {
        params["arguments"]["wait_for_millis"]
            .as_u64()
            .filter(|value| *value <= 120_000)
            .ok_or_else(|| String::from("MCP transition wait is outside its bound"))?
    } else {
        0
    };
    Ok(std::time::Duration::from_millis(wait + 5_000))
}
