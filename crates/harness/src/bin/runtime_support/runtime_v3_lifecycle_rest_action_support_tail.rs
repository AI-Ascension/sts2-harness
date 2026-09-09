// SPDX-License-Identifier: MIT

fn simple_response(
    file: &str,
    correlation_id: &str,
    state_id: &str,
    generation: u64,
    operation_id: &str,
    action_id: &str,
    action: Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    let mut value: Value = match file {
        "unknown" => serde_json::from_str(include_str!(
            "../../../../../protocol-artifact/runtime-v4-expert-rest-action/golden/action-unknown.json"
        ))?,
        "accepted" => serde_json::from_str(include_str!(
            "../../../../../protocol-artifact/runtime-v4-expert-rest-action/golden/action-accepted.json"
        ))?,
        "cancelled" => {
            let mut value: Value = serde_json::from_str(include_str!(
                "../../../../../protocol-artifact/runtime-v4-expert-rest-action/golden/action-rejected.json"
            ))?;
            value["status"] = json!("cancelled");
            value["error_code"] = json!("sts2.game-mod/selection_cancelled");
            value
        }
        _ => return Err(format!("unsupported REST response fixture {file}").into()),
    };
    set_response_identity(
        &mut value,
        correlation_id,
        state_id,
        generation,
        operation_id,
        action_id,
        action,
    );
    Ok(value)
}

#[allow(clippy::too_many_arguments)]
fn settled_selection_requested(
    correlation_id: &str,
    state_id: &str,
    generation: u64,
    before_generation: u64,
    operation_id: &str,
    action_id: &str,
    option: &str,
    selection_kind: &str,
    selection_id: &str,
    choices: Value,
    selector_actions: Value,
    observation_actions: Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    let mut value: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/runtime-v4-expert-rest-action/golden/action-selection-requested.json"
    ))?;
    set_response_identity(
        &mut value,
        correlation_id,
        state_id,
        generation,
        operation_id,
        action_id,
        json!({"kind":"rest_option","rest_option_id":option}),
    );
    value["observation"]["state_id"] = json!(state_id);
    value["observation"]["generation"] = json!(generation);
    value["observation"]["state"] = json!({"state":"selection","choices":choices});
    value["observation"]["legal_actions"] = observation_actions;
    value["transition"]["before_generation"] = json!(before_generation);
    value["transition"]["after_generation"] = json!(generation);
    value["transition"]["rest_option_id"] = json!(option);
    value["transition"]["selector"] = json!({
        "selection_id":selection_id,
        "selection_kind":selection_kind,
        "required_count":if selection_kind == "player" { 1 } else { 2 },
        "selected_choice_ids":[],
        "remaining_count":if selection_kind == "player" { 1 } else { 2 },
        "legal_actions":selector_actions
    });
    Ok(value)
}

#[allow(clippy::too_many_arguments)]
fn settled_progressed(
    file: &str,
    correlation_id: &str,
    state_id: &str,
    generation: u64,
    before_generation: u64,
    operation_id: &str,
    action_id: &str,
    action: Value,
    observation_actions: Value,
    selector_actions: Value,
    selected_choice_ids: Value,
    remaining_count: u64,
) -> Result<Value, Box<dyn std::error::Error>> {
    let mut value: Value = match file {
        "progressed" => serde_json::from_str(include_str!(
            "../../../../../protocol-artifact/runtime-v4-expert-rest-action/golden/action-selection-progressed.json"
        ))?,
        "completed" => serde_json::from_str(include_str!(
            "../../../../../protocol-artifact/runtime-v4-expert-rest-action/golden/action-selection-completed.json"
        ))?,
        _ => return Err(format!("unsupported settled fixture {file}").into()),
    };
    set_response_identity(
        &mut value,
        correlation_id,
        state_id,
        generation,
        operation_id,
        action_id,
        action,
    );
    value["observation"]["state_id"] = json!(state_id);
    value["observation"]["generation"] = json!(generation);
    value["observation"]["legal_actions"] = observation_actions;
    value["transition"]["before_generation"] = json!(before_generation);
    value["transition"]["after_generation"] = json!(generation);
    value["transition"]["selected_choice_ids"] = selected_choice_ids.clone();
    value["transition"]["remaining_count"] = json!(remaining_count);
    if file == "progressed" {
        value["transition"]["selector"]["selection_id"] = json!("selection:10:smith");
        value["transition"]["selector"]["selected_choice_ids"] = selected_choice_ids;
        value["transition"]["selector"]["remaining_count"] = json!(remaining_count);
        value["transition"]["selector"]["legal_actions"] = selector_actions;
    }
    Ok(value)
}
