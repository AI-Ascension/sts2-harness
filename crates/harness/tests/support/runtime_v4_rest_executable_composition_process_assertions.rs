// SPDX-License-Identifier: MIT

pub(crate) fn assert_success(
    result: &ScenarioResult,
) -> Result<String, Box<dyn std::error::Error>> {
    if result.runtime.status.code() != Some(0) {
        let native = result
            .ledger
            .requests
            .iter()
            .zip(&result.ledger.responses)
            .map(|(request, response)| {
                json!({
                    "method": request.method,
                    "path": request.path,
                    "operation_id": request.body["operation_id"],
                    "action_id": request.body["action"]["action_id"],
                    "generation": request.body["generation"],
                    "response_status": response.status,
                    "response": response.body
                })
            })
            .collect::<Vec<_>>();
        return Err(format!(
            "runtime failed: {}; gateway stderr: {}; native ledger: {}",
            String::from_utf8_lossy(&result.runtime.stderr),
            String::from_utf8_lossy(&result.gateway.stderr),
            serde_json::to_string(&native)?
        )
        .into());
    }
    if output_was_truncated(&result.runtime.stdout)
        || output_was_truncated(&result.runtime.stderr)
        || output_was_truncated(&result.gateway.stdout)
        || output_was_truncated(&result.gateway.stderr)
    {
        return Err("bounded executable composition output was truncated".into());
    }
    if !result.ledger.errors.is_empty() {
        return Err(format!("fixture failed: {:?}", result.ledger.errors).into());
    }
    let posts = action_requests(&result.ledger);
    if posts.len() != 6 || posts.iter().any(|request| request.method != "POST") {
        return Err(format!(
            "expected six REST action requests using POST, got {}",
            posts.len()
        )
        .into());
    }
    let action_ids: Vec<&str> = posts
        .iter()
        .map(|request| {
            request.body["action"]["action_id"]
                .as_str()
                .unwrap_or("<missing>")
        })
        .collect();
    let expected_actions = [
        "rest-option:9:smith",
        "select_card:10:smith:card:1",
        "select_card:11:smith:card:2",
        "confirm_selection:12:smith",
        "rest-option:13:mend",
        "select_player:14:mend:player:local",
    ];
    if action_ids != expected_actions {
        return Err(format!("unexpected REST action sequence: {action_ids:?}").into());
    }
    let statuses: Vec<u16> = result
        .ledger
        .responses
        .iter()
        .zip(&result.ledger.requests)
        .filter_map(|(response, request)| {
            (request.method == "POST"
                && request.path == "/api/v4/runtime/expert-rest-action")
                .then_some(response.status)
        })
        .collect();
    if statuses != [503, 202, 200, 200, 503, 200] {
        return Err(format!("unexpected REST action statuses: {statuses:?}").into());
    }
    let reconciles: Vec<_> = result
        .ledger
        .requests
        .iter()
        .zip(&result.ledger.responses)
        .filter(|(request, _)| {
            request
                .path
            .starts_with("/api/v4/runtime/expert-rest-actions/")
        })
        .collect();
    if reconciles.len() != 3
        || reconciles
            .iter()
            .any(|(request, response)| {
                request.method != "GET" || !request.body.is_null() || response.status != 200
            })
    {
        return Err(format!("unexpected REST reconciliation requests: {reconciles:?}").into());
    }
    let operation_ids: Vec<&str> = posts
        .iter()
        .map(|request| request.body["operation_id"].as_str().unwrap_or("<missing>"))
        .collect();
    if operation_ids
        != [
            "episode-action-9-1",
            "episode-action-10-2",
            "episode-action-11-3",
            "episode-action-12-4",
            "episode-action-13-5",
            "episode-action-14-6",
        ]
    {
        return Err(format!("unexpected REST operation sequence: {operation_ids:?}").into());
    }
    let reconcile_operation_ids: Vec<&str> = reconciles
        .iter()
        .map(|(request, _)| {
            request
                .path
                .strip_prefix("/api/v4/runtime/expert-rest-actions/")
                .unwrap_or("<missing>")
        })
        .collect();
    if reconcile_operation_ids != [operation_ids[0], operation_ids[1], operation_ids[4]] {
        return Err(format!(
            "reconciliation did not retain original operation IDs: {reconcile_operation_ids:?}"
        )
        .into());
    }
    for ((_, response), expected_operation, expected_generation, expected_state) in [
        (&reconciles[0], operation_ids[0], 10, "live:10"),
        (&reconciles[1], operation_ids[1], 11, "live:11"),
        (&reconciles[2], operation_ids[4], 14, "live:14"),
    ] {
        if response.body["status"] != "settled"
            || response.body["operation_id"] != expected_operation
            || response.body["generation"] != expected_generation
            || response.body["state_id"] != expected_state
            || response.body["observation"]["generation"] != expected_generation
            || response.body["observation"]["state_id"] != expected_state
        {
            return Err(format!(
                "durable REST receipt lost identity for {expected_operation}: {}",
                response.body
            )
            .into());
        }
    }
    let progress = &reconciles[1].1.body;
    if progress["action"]["action_id"] != action_ids[1]
        || progress["transition"]["kind"] != "rest_option_selection_progressed"
        || progress["transition"]["before_generation"] != 10
        || progress["transition"]["after_generation"] != 11
        || progress["transition"]["selected_choice_ids"] != json!(["card:1"])
        || progress["transition"]["effect_witness"] != Value::Null
        || progress["effect_witness"] != Value::Null
    {
        return Err(format!(
            "accepted REST progress lost operation identity or witness boundary: {progress}"
        )
        .into());
    }
    let native_response = |index: usize| {
        result
            .ledger
            .responses
            .iter()
            .zip(&result.ledger.requests)
            .filter(|(_, request)| {
                request.method == "POST" && request.path == "/api/v4/runtime/expert-rest-action"
            })
            .nth(index)
            .map(|(response, _)| response)
    };
    for (index, expected_operation, expected_generation, expected_state) in [
        (0, operation_ids[0], 9, "live:9"),
        (4, operation_ids[4], 13, "live:13"),
    ] {
        let response = native_response(index).ok_or("missing native unknown response")?;
        if response.status != 503
            || response.body["status"] != "unknown"
            || response.body["operation_id"] != expected_operation
            || response.body["generation"] != expected_generation
            || response.body["state_id"] != expected_state
        {
            return Err(format!(
                "unknown REST receipt lost original operation identity: {}",
                response.body
            )
            .into());
        }
    }
    for (index, expected_kind, expected_operation, expected_generation) in [
        (3, "smith_applied", operation_ids[3], 13),
        (5, "mend_applied", operation_ids[5], 15),
    ] {
        let response = native_response(index).ok_or("missing native settled response")?;
        if response.status != 200
            || response.body["status"] != "settled"
            || response.body["effect_witness"]["kind"] != expected_kind
            || response.body["effect_witness"]["operation_id"] != expected_operation
            || response.body["effect_witness"]["generation"] != expected_generation
        {
            return Err(format!(
                "REST effect witness was not durable for {expected_operation}: {}",
                response.body
            )
            .into());
        }
    }
    if operation_ids.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err("operation IDs were reused".into());
    }
    if posts.iter().any(|request| {
        request.body["protocol_version"] != "runtime-v4-expert-rest-action-v1"
            || request.body["profile"] != "expert-rest-action"
            || request.body["schema_digest"] != REST_SCHEMA_DIGEST
            || request.body["instance_id"] != INSTANCE_ID
            || request.body["session_id"] != SESSION_ID
            || request.body["lease_id"] != LEASE_ID
            || request.body["lease_epoch"] != LEASE_EPOCH
            || request.body["status"] != Value::Null
    }) {
        return Err("REST action request envelope was not canonical".into());
    }
    let operation = operation_ids[0].to_owned();
    assert_persisted_receipts(result)?;
    let report = completion_report(&result.runtime.stderr)?;
    if report["protocol"] != "runtime-v4-expert-rest-action"
        || report["status"] != "complete"
        || report["terminal_stage"] != "victory"
        || report["final_generation"] != 15
        || report["transitions"] != 6
        || report["recoveries"] != 2
    {
        return Err(format!("runtime completion report mismatch: {report}").into());
    }
    let response_statuses: Vec<u16> = result
        .ledger
        .responses
        .iter()
        .map(|response| response.status)
        .collect();
    if response_statuses
        .iter()
        .filter(|status| **status == 503)
        .count()
        != 2
    {
        return Err(format!("native unknown status count mismatch: {response_statuses:?}").into());
    }
    Ok(operation)
}
