// SPDX-License-Identifier: MIT

fn fixture_response(
    state: &mut FixtureState,
    request: &DownstreamRequest,
) -> Result<(u16, Value), String> {
    validate_headers(request)?;
    match request.path.as_str() {
        "/api/v3/runtime/state" => Ok((200, v3_response("state_response", request, state.phase)?)),
        "/api/v3/runtime/legal-actions" => Ok((
            200,
            v3_response("legal_actions_response", request, state.phase)?,
        )),
        "/api/v4/runtime/expert-state" => Ok((200, expert_state(state.phase)?)),
        "/api/v4/runtime/expert-rest-action" => rest_action_post(state, request),
        path if path.starts_with("/api/v4/runtime/expert-rest-actions/") => {
            rest_action_get(state, request, path)
        }
        _ => Err(format!("unexpected native path {}", request.path)),
    }
}

fn validate_headers(request: &DownstreamRequest) -> Result<(), String> {
    if request.headers.get("authorization").map(String::as_str) != Some("Bearer mod-token") {
        return Err(String::from("native fixture authorization mismatch"));
    }
    for (name, expected) in [
        ("x-sts2-instance-id", INSTANCE_ID),
        ("x-sts2-caller-id", CALLER_ID),
        ("x-sts2-session-id", SESSION_ID),
        ("x-sts2-lease-id", LEASE_ID),
        ("x-sts2-lease-epoch", "1"),
    ] {
        if request.headers.get(name).map(String::as_str) != Some(expected) {
            return Err(format!("native fixture identity header {name} mismatch"));
        }
    }
    if !request.headers.contains_key("x-sts2-correlation-id") {
        return Err(String::from("native fixture correlation header is missing"));
    }
    Ok(())
}

fn rest_action_post(
    state: &mut FixtureState,
    request: &DownstreamRequest,
) -> Result<(u16, Value), String> {
    if request.method != "POST" {
        return Err(format!(
            "REST action endpoint requires POST, got {}",
            request.method
        ));
    }
    let body = request
        .body
        .as_object()
        .ok_or_else(|| String::from("REST action POST body is not an object"))?;
    for (field, expected) in [
        ("protocol_version", "runtime-v4-expert-rest-action-v1"),
        ("profile", "expert-rest-action"),
        ("kind", "action_request"),
        ("instance_id", INSTANCE_ID),
        ("session_id", SESSION_ID),
        ("lease_id", LEASE_ID),
    ] {
        if body.get(field).and_then(Value::as_str) != Some(expected) {
            return Err(format!("REST action request {field} mismatch"));
        }
    }
    if body["schema_digest"] != REST_SCHEMA_DIGEST
        || body["lease_epoch"] != LEASE_EPOCH
        || !body["status"].is_null()
        || !body["observation"].is_null()
        || !body["transition"].is_null()
        || !body["effect_witness"].is_null()
        || !body["error_code"].is_null()
    {
        return Err(String::from(
            "REST action request envelope is not canonical",
        ));
    }
    let operation_id = body["operation_id"]
        .as_str()
        .ok_or_else(|| String::from("REST action operation is missing"))?;
    let generation = body["generation"]
        .as_u64()
        .ok_or_else(|| String::from("REST action generation is missing"))?;
    let state_id = body["state_id"]
        .as_str()
        .ok_or_else(|| String::from("REST action state identity is missing"))?;
    let action = body
        .get("action")
        .cloned()
        .ok_or_else(|| String::from("REST action payload is missing"))?;
    let action_id = action["action_id"]
        .as_str()
        .ok_or_else(|| String::from("REST action ID is missing"))?
        .to_owned();
    if state.operations.contains_key(operation_id) {
        state.duplicate_posts += 1;
        return Err(format!("duplicate REST action POST for {operation_id}"));
    }
    let operation = Operation {
        generation,
        state_id: state_id.to_owned(),
        action: action.clone(),
        action_id: action_id.clone(),
        unknown: false,
        settlement: None,
    };
    let (status, response, unknown, settlement) = match (state.phase, action_id.as_str()) {
        (Phase::SmithOption, "rest-option:9:smith") => {
            (503, unknown_response(&body_value(body))?, true, None)
        }
        (Phase::SmithSelection(0), "select_card:10:smith:card:1") => {
            state.phase = Phase::SmithSelection(1);
            (
                202,
                accepted_response(&body_value(body))?,
                false,
                Some(Settlement::SmithCardOne),
            )
        }
        (Phase::SmithSelection(1), "select_card:11:smith:card:2") => {
            state.phase = Phase::SmithSelection(2);
            (
                200,
                smith_response(&body_value(body), 2)?,
                false,
                Some(Settlement::SmithCardTwo),
            )
        }
        (Phase::SmithSelection(2), "confirm_selection:12:smith") => {
            state.phase = Phase::MendOption;
            (
                200,
                smith_response(&body_value(body), 3)?,
                false,
                Some(Settlement::SmithCompleted),
            )
        }
        (Phase::MendOption, "rest-option:13:mend") => {
            (503, unknown_response(&body_value(body))?, true, None)
        }
        (Phase::MendSelection, "select_player:14:mend:player:local") => {
            state.phase = Phase::Victory;
            (
                200,
                mend_response(&body_value(body), 1)?,
                false,
                Some(Settlement::MendCompleted),
            )
        }
        _ => {
            return Err(format!(
                "REST action {} is illegal in phase {:?}",
                action_id, state.phase
            ));
        }
    };
    let mut operation = operation;
    operation.unknown = unknown;
    operation.settlement = settlement;
    state.operations.insert(operation_id.to_owned(), operation);
    Ok((status, response))
}

fn rest_action_get(
    state: &mut FixtureState,
    request: &DownstreamRequest,
    path: &str,
) -> Result<(u16, Value), String> {
    if request.method != "GET" {
        return Err(format!(
            "REST reconciliation endpoint requires GET, got {}",
            request.method
        ));
    }
    if path.contains('?') {
        return Err(String::from(
            "REST reconciliation endpoint does not accept a query string",
        ));
    }
    if !request.body.is_null() {
        return Err(String::from("REST reconcile GET carried a request body"));
    }
    let operation_id = path
        .strip_prefix("/api/v4/runtime/expert-rest-actions/")
        .filter(|id| !id.is_empty())
        .ok_or_else(|| String::from("REST reconcile operation is missing"))?;
    let operation = state
        .operations
        .get(operation_id)
        .cloned()
        .ok_or_else(|| format!("REST reconcile referenced unknown operation {operation_id}"))?;
    if operation.settlement == Some(Settlement::SmithCardOne) {
        return Ok((
            200,
            smith_response(&reconcile_body(request, operation_id, &operation), 1)?,
        ));
    }
    if !operation.unknown {
        return Err(format!(
            "unexpected reconciliation for settled operation {operation_id}"
        ));
    }
    let settlement = match operation.action_id.as_str() {
        "rest-option:9:smith" => {
            state.phase = Phase::SmithSelection(0);
            Settlement::SmithRequested
        }
        "rest-option:13:mend" => {
            state.phase = Phase::MendSelection;
            Settlement::MendRequested
        }
        _ => return Err(format!("unknown operation action {}", operation.action_id)),
    };
    if let Some(record) = state.operations.get_mut(operation_id) {
        record.settlement = Some(settlement);
    }
    let body = reconcile_body(request, operation_id, &operation);
    let response = match settlement {
        Settlement::SmithRequested => smith_response(&body, 0)?,
        Settlement::MendRequested => mend_response(&body, 0)?,
        _ => return Err(String::from("invalid unknown operation settlement")),
    };
    Ok((200, response))
}

fn body_value(body: &serde_json::Map<String, Value>) -> Value {
    Value::Object(body.clone())
}

fn reconcile_body(request: &DownstreamRequest, operation_id: &str, operation: &Operation) -> Value {
    json!({
        "correlation_id": request.headers.get("x-sts2-correlation-id").cloned().unwrap_or_default(),
        "instance_id": INSTANCE_ID,
        "session_id": SESSION_ID,
        "lease_id": LEASE_ID,
        "lease_epoch": LEASE_EPOCH,
        "generation": operation.generation,
        "state_id": operation.state_id,
        "operation_id": operation_id,
        "action": operation.action
    })
}

fn unknown_response(request: &Value) -> Result<Value, String> {
    let mut response = golden("action-unknown.json")?;
    set_result_identity(
        &mut response,
        request,
        request["generation"].clone(),
        request["state_id"].clone(),
    );
    Ok(response)
}

fn accepted_response(request: &Value) -> Result<Value, String> {
    let mut response = golden("action-accepted.json")?;
    set_result_identity(
        &mut response,
        request,
        request["generation"].clone(),
        request["state_id"].clone(),
    );
    Ok(response)
}

fn smith_response(request: &Value, step: usize) -> Result<Value, String> {
    if step > 3 {
        return Err(format!("unsupported Smith producer-vector step {step}"));
    }
    let mut response = producer_response("smith-selection-lifecycle.json", step)?;
    let (after_generation, after_state, before_generation) = match step {
        0 => (10, "live:10", 9),
        1 => (11, "live:11", 10),
        2 => (12, "live:12", 11),
        3 => (13, "live:13", 12),
        _ => unreachable!(),
    };
    set_result_identity(
        &mut response,
        request,
        json!(after_generation),
        json!(after_state),
    );
    response["observation"] = expert_observation(match step {
        0..=2 => Phase::SmithSelection(step as u8),
        _ => Phase::MendOption,
    })?;
    response["observation"]["state_id"] = json!(after_state);
    response["observation"]["generation"] = json!(after_generation);
    response["transition"]["before_generation"] = json!(before_generation);
    response["transition"]["after_generation"] = json!(after_generation);
    response["transition"]["rest_option_id"] = json!("smith");
    match step {
        0 => {
            response["transition"]["selector"] = smith_selector(0);
        }
        1 => {
            response["transition"]["selection_id"] = json!("selection:10:smith");
            response["transition"]["selection_kind"] = json!("card");
            response["transition"]["required_count"] = json!(2);
            response["transition"]["selected_choice_ids"] = json!(["card:1"]);
            response["transition"]["remaining_count"] = json!(1);
            response["transition"]["selector"] = smith_selector(1);
        }
        2 => {
            response["transition"]["selection_id"] = json!("selection:10:smith");
            response["transition"]["selection_kind"] = json!("card");
            response["transition"]["required_count"] = json!(2);
            response["transition"]["selected_choice_ids"] = json!(["card:1", "card:2"]);
            response["transition"]["remaining_count"] = json!(0);
            response["transition"]["selector"] = smith_selector(2);
        }
        3 => {
            response["transition"]["selection_id"] = json!("selection:10:smith");
            response["transition"]["selection_kind"] = json!("card");
            response["transition"]["required_count"] = json!(2);
            response["transition"]["selected_choice_ids"] = json!(["card:1", "card:2"]);
            response["transition"]["remaining_count"] = json!(0);
            if let Some(transition) = response["transition"].as_object_mut() {
                transition.remove("selector");
            }
            set_effect_identity(
                &mut response,
                request["operation_id"].clone(),
                after_generation,
            );
        }
        _ => unreachable!(),
    }
    Ok(response)
}

include!("runtime_v4_rest_executable_composition_fixture_tail.rs");
