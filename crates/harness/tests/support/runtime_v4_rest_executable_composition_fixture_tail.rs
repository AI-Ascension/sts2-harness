// SPDX-License-Identifier: MIT

fn mend_response(
    request: &Value,
    step: usize,
    selector_encoding: SelectorEncoding,
) -> Result<Value, String> {
    if step > 1 {
        return Err(format!("unsupported Mend producer-vector step {step}"));
    }
    let mut response = producer_response("mend-selection-lifecycle.json", step)?;
    let (after_generation, after_state, before_generation) = if step == 0 {
        (14, "live:14", 13)
    } else {
        (15, "live:15", 14)
    };
    set_result_identity(
        &mut response,
        request,
        json!(after_generation),
        json!(after_state),
    );
    response["observation"] = expert_observation(
        if step == 0 {
            Phase::MendSelection
        } else {
            Phase::Victory
        },
        selector_encoding,
    )?;
    response["observation"]["state_id"] = json!(after_state);
    response["observation"]["generation"] = json!(after_generation);
    response["transition"]["before_generation"] = json!(before_generation);
    response["transition"]["after_generation"] = json!(after_generation);
    response["transition"]["rest_option_id"] = json!("mend");
    if step == 0 {
        response["transition"]["selector"] = mend_selector(selector_encoding);
    } else {
        response["transition"]["selection_id"] = json!("selection:14:mend");
        response["transition"]["selection_kind"] = json!("player");
        response["transition"]["required_count"] = json!(1);
        response["transition"]["selected_choice_ids"] = json!(["player:local"]);
        response["transition"]["remaining_count"] = json!(0);
        set_effect_identity(
            &mut response,
            request["operation_id"].clone(),
            after_generation,
        );
    }
    Ok(response)
}

fn set_result_identity(response: &mut Value, request: &Value, generation: Value, state_id: Value) {
    for (field, value) in [
        ("correlation_id", request["correlation_id"].clone()),
        ("instance_id", json!(INSTANCE_ID)),
        ("session_id", json!(SESSION_ID)),
        ("lease_id", json!(LEASE_ID)),
        ("lease_epoch", json!(LEASE_EPOCH)),
        ("generation", generation),
        ("state_id", state_id),
        ("operation_id", request["operation_id"].clone()),
        ("action", request["action"].clone()),
    ] {
        response[field] = value;
    }
}

fn set_effect_identity(response: &mut Value, operation_id: Value, generation: u64) {
    if let Some(effect) = response.get_mut("effect_witness") {
        set_one_effect_identity(effect, &operation_id, generation);
    }
    if let Some(effect) = response
        .get_mut("transition")
        .and_then(|transition| transition.get_mut("effect_witness"))
    {
        set_one_effect_identity(effect, &operation_id, generation);
    }
}

fn set_one_effect_identity(effect: &mut Value, operation_id: &Value, generation: u64) {
    if effect.is_object() {
        effect["operation_id"] = operation_id.clone();
        effect["generation"] = json!(generation);
    }
}

fn producer_response(file: &str, index: usize) -> Result<Value, String> {
    let text = match file {
        "smith-selection-lifecycle.json" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../protocol-artifact/runtime-v4-expert-rest-action/producer/smith-selection-lifecycle.json"
        )),
        "mend-selection-lifecycle.json" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../protocol-artifact/runtime-v4-expert-rest-action/producer/mend-selection-lifecycle.json"
        )),
        _ => return Err(format!("unknown producer vector {file}")),
    };
    let root: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    root["messages"]
        .as_array()
        .and_then(|messages| messages.get(index))
        .and_then(|message| message.get("response"))
        .cloned()
        .ok_or_else(|| format!("producer vector {file} response {index} is missing"))
}

fn golden(file: &str) -> Result<Value, String> {
    let text = match file {
        "action-unknown.json" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../protocol-artifact/runtime-v4-expert-rest-action/golden/action-unknown.json"
        )),
        "action-accepted.json" => include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../protocol-artifact/runtime-v4-expert-rest-action/golden/action-accepted.json"
        )),
        _ => return Err(format!("unknown REST golden {file}")),
    };
    serde_json::from_str(text).map_err(|error| error.to_string())
}

fn expert_state(phase: Phase, selector_encoding: SelectorEncoding) -> Result<Value, String> {
    expert_observation(phase, selector_encoding)
}

fn expert_observation(
    phase: Phase,
    selector_encoding: SelectorEncoding,
) -> Result<Value, String> {
    let mut value: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../protocol-artifact/runtime-v4-expert/golden/observation.json"
    )))
    .map_err(|error| error.to_string())?;
    let (state_id, generation) = phase.identity();
    value["state_id"] = json!(state_id);
    value["generation"] = json!(generation);
    value["run"]["location"] = json!("rest:1");
    value["state"] = match phase {
        Phase::SmithOption => json!({
            "state":"rest",
            "choices":[
                {"choice_id":"rest:smith","label":"Smith","kind":"rest","domain":null},
                {"choice_id":"rest:mend","label":"Mend","kind":"rest","domain":null}
            ]
        }),
        Phase::SmithSelection(_) => json!({
            "state":"selection",
            "choices":[
                {"choice_id":"card:1","label":"Strike","kind":"selection","domain":null},
                {"choice_id":"card:2","label":"Bash","kind":"selection","domain":null}
            ]
        }),
        Phase::MendOption => json!({
            "state":"rest",
            "choices":[{"choice_id":"rest:mend","label":"Mend","kind":"rest","domain":null}]
        }),
        Phase::MendSelection => json!({
            "state":"selection",
            "choices":[{"choice_id":"player:local","label":"Ironclad","kind":"selection","domain":null}]
        }),
        Phase::Victory => json!({"state":"victory"}),
    };
    value["legal_actions"] = expert_legal_actions(phase, selector_encoding);
    if matches!(phase, Phase::Victory) {
        value["player"]["potions"] = json!([]);
    }
    Ok(value)
}

fn expert_legal_actions(phase: Phase, selector_encoding: SelectorEncoding) -> Value {
    match phase {
        Phase::SmithOption => json!([
            {"action_id":"rest-option:9:smith","action":{"kind":"rest_option","rest_option_id":"smith"}},
            {"action_id":"rest-option:9:mend","action":{"kind":"rest_option","rest_option_id":"mend"}}
        ]),
        Phase::SmithSelection(selected) => match selected {
            0 => json!([
                {"action_id":selector_encoding.action_id(10, "selection:10:smith", "smith", "select_card", Some("card:1")), "action":{"kind":"select_card","selection_id":null,"card_id":"card:1"}},
                {"action_id":selector_encoding.action_id(10, "selection:10:smith", "smith", "select_card", Some("card:2")), "action":{"kind":"select_card","selection_id":null,"card_id":"card:2"}},
                {"action_id":selector_encoding.action_id(10, "selection:10:smith", "smith", "confirm_selection", None), "action":{"kind":"confirm_selection","selection_id":null}},
                {"action_id":selector_encoding.action_id(10, "selection:10:smith", "smith", "cancel_selection", None), "action":{"kind":"cancel_selection","selection_id":null}}
            ]),
            1 => json!([
                {"action_id":selector_encoding.action_id(11, "selection:10:smith", "smith", "select_card", Some("card:2")), "action":{"kind":"select_card","selection_id":null,"card_id":"card:2"}},
                {"action_id":selector_encoding.action_id(11, "selection:10:smith", "smith", "confirm_selection", None), "action":{"kind":"confirm_selection","selection_id":null}},
                {"action_id":selector_encoding.action_id(11, "selection:10:smith", "smith", "cancel_selection", None), "action":{"kind":"cancel_selection","selection_id":null}}
            ]),
            _ => json!([
                {"action_id":selector_encoding.action_id(12, "selection:10:smith", "smith", "confirm_selection", None), "action":{"kind":"confirm_selection","selection_id":null}},
                {"action_id":selector_encoding.action_id(12, "selection:10:smith", "smith", "cancel_selection", None), "action":{"kind":"cancel_selection","selection_id":null}}
            ]),
        },
        Phase::MendOption => json!([
            {"action_id":"rest-option:13:mend","action":{"kind":"rest_option","rest_option_id":"mend"}}
        ]),
        Phase::MendSelection => json!([
            {"action_id":selector_encoding.action_id(14, "selection:14:mend", "mend", "cancel_selection", None), "action":{"kind":"cancel_selection","selection_id":null}}
        ]),
        Phase::Victory => json!([]),
    }
}

fn smith_selector(selected: u8, selector_encoding: SelectorEncoding) -> Value {
    let actions = match selected {
        0 => json!([
            selector_action(selector_encoding, 10, "selection:10:smith", "smith", "select_card", Some("card:1")),
            selector_action(selector_encoding, 10, "selection:10:smith", "smith", "select_card", Some("card:2")),
            selector_action(selector_encoding, 10, "selection:10:smith", "smith", "cancel_selection", None)
        ]),
        1 => json!([
            selector_action(selector_encoding, 11, "selection:10:smith", "smith", "select_card", Some("card:2")),
            selector_action(selector_encoding, 11, "selection:10:smith", "smith", "cancel_selection", None)
        ]),
        _ => json!([
            selector_action(selector_encoding, 12, "selection:10:smith", "smith", "confirm_selection", None),
            selector_action(selector_encoding, 12, "selection:10:smith", "smith", "cancel_selection", None)
        ]),
    };
    json!({
        "selection_id":"selection:10:smith",
        "selection_kind":"card",
        "required_count":2,
        "selected_choice_ids":match selected { 0 => json!([]), 1 => json!(["card:1"]), _ => json!(["card:1","card:2"]) },
        "remaining_count":u64::from(2_u8.saturating_sub(selected)),
        "legal_actions":actions
    })
}

fn mend_selector(selector_encoding: SelectorEncoding) -> Value {
    json!({
        "selection_id":"selection:14:mend",
        "selection_kind":"player",
        "required_count":1,
        "selected_choice_ids":[],
        "remaining_count":1,
        "legal_actions":[
            selector_action(selector_encoding, 14, "selection:14:mend", "mend", "select_player", Some("player:local")),
            selector_action(selector_encoding, 14, "selection:14:mend", "mend", "cancel_selection", None)
        ]
    })
}

fn selector_action(
    selector_encoding: SelectorEncoding,
    generation: u64,
    selection_id: &str,
    option_id: &str,
    kind: &str,
    choice: Option<&str>,
) -> Value {
    let action_id = selector_encoding.action_id(
        generation,
        selection_id,
        option_id,
        kind,
        choice,
    );
    let action = match kind {
        "select_card" => {
            json!({"kind":"select_card","selection_id":selection_id,"rest_option_id":option_id,"card_id":choice.unwrap_or("")})
        }
        "select_player" => {
            json!({"kind":"select_player","selection_id":selection_id,"rest_option_id":option_id,"player_id":choice.unwrap_or("")})
        }
        "confirm_selection" | "cancel_selection" => {
            json!({"kind":kind,"selection_id":selection_id,"rest_option_id":option_id})
        }
        _ => Value::Null,
    };
    json!({"action_id":action_id,"action":action})
}

fn v3_response(kind: &str, request: &DownstreamRequest, phase: Phase) -> Result<Value, String> {
    let mut value: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../protocol-artifact/runtime-v3-gameplay/golden/state-response.json"
    )))
    .map_err(|error| error.to_string())?;
    let (state_id, generation) = phase.identity();
    value["kind"] = json!(kind);
    value["correlation_id"] = json!(
        request
            .headers
            .get("x-sts2-correlation-id")
            .cloned()
            .unwrap_or_default()
    );
    value["instance_id"] = json!(INSTANCE_ID);
    value["session_id"] = json!(SESSION_ID);
    value["lease_id"] = json!(LEASE_ID);
    value["lease_epoch"] = json!(LEASE_EPOCH);
    value["generation"] = json!(generation);
    value["state_id"] = json!(state_id);
    value["observation"]["state_id"] = json!(state_id);
    value["observation"]["generation"] = json!(generation);
    value["observation"]["state"] = v3_state(phase);
    value["legal_actions"] = json!([]);
    if kind == "legal_actions_response" {
        value["observation"] = Value::Null;
    }
    Ok(value)
}

fn v3_state(phase: Phase) -> Value {
    match phase {
        Phase::SmithOption => json!({"state":"rest","options":["smith","mend"]}),
        Phase::SmithSelection(_) => json!({"state":"selection","choices":["card:1","card:2"]}),
        Phase::MendOption => json!({"state":"rest","options":["mend"]}),
        Phase::MendSelection => json!({"state":"selection","choices":["player:local"]}),
        Phase::Victory => json!({"state":"victory"}),
    }
}

fn read_request(stream: &mut TcpStream) -> Result<DownstreamRequest, Box<dyn std::error::Error>> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut bytes = Vec::new();
    let header_end = loop {
        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break end;
        }
        if bytes.len() > 64 * 1024 {
            return Err("native request header bound exceeded".into());
        }
        let mut chunk = [0_u8; 2048];
        let count = stream.read(&mut chunk)?;
        if count == 0 {
            return Err("native request ended before headers".into());
        }
        bytes.extend_from_slice(&chunk[..count]);
    };
    let mut lines = std::str::from_utf8(&bytes[..header_end])?.split("\r\n");
    let request_line = lines.next().ok_or("native request line missing")?;
    let mut parts = request_line.split_ascii_whitespace();
    let method = parts.next().ok_or("native method missing")?.to_owned();
    let path = parts.next().ok_or("native path missing")?.to_owned();
    if parts.next() != Some("HTTP/1.1") || parts.next().is_some() {
        return Err("native request line invalid".into());
    }
    let mut headers = BTreeMap::new();
    for line in lines {
        let (name, value) = line.split_once(':').ok_or("native header invalid")?;
        headers.insert(name.to_ascii_lowercase(), value.trim().to_owned());
    }
    let length = headers
        .get("content-length")
        .ok_or("native content length missing")?
        .parse::<usize>()?;
    if length > 256 * 1024 {
        return Err("native request body bound exceeded".into());
    }
    let body_start = header_end + 4;
    if bytes.len().saturating_sub(body_start) > length {
        return Err("native request has trailing bytes".into());
    }
    let mut body = bytes[body_start..].to_vec();
    while body.len() < length {
        let mut chunk = vec![0_u8; (length - body.len()).min(2048)];
        let count = stream.read(&mut chunk)?;
        if count == 0 {
            return Err("native request ended before body".into());
        }
        body.extend_from_slice(&chunk[..count]);
    }
    Ok(DownstreamRequest {
        method,
        path,
        headers,
        body: if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&body)?
        },
    })
}

fn write_response(stream: &mut TcpStream, status: u16, body: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(body).map_err(std::io::Error::other)?;
    let headers = format!(
        "HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(headers.as_bytes())?;
    stream.write_all(&body)
}
