// SPDX-License-Identifier: MIT

fn rest_response_sets() -> Result<(Vec<Value>, Vec<Value>), Box<dyn std::error::Error>> {
    let initial_rest = expert_state(
        "live:9",
        9,
        "rest",
        json!([
            rest_option("rest-option:9:smith", "smith"),
            rest_option("rest-option:9:mend", "mend")
        ]),
    )?;
    let selection_requested = settled_selection_requested(
        "4",
        "live:10",
        10,
        9,
        "rest-op:9:smith",
        "rest-option:9:smith",
        "smith",
        "card",
        "selection:10:smith",
        json!([
            {"choice_id":"card:1","label":"Strike","kind":"selection","domain":null},
            {"choice_id":"card:2","label":"Bash","kind":"selection","domain":null}
        ]),
        json!([
            selector_action(
                "select_card:10:smith:card:1",
                "select_card",
                "selection:10:smith",
                "smith",
                Some("card:1")
            ),
            selector_action(
                "select_card:10:smith:card:2",
                "select_card",
                "selection:10:smith",
                "smith",
                Some("card:2")
            ),
            selector_action(
                "cancel_selection:10:smith",
                "cancel_selection",
                "selection:10:smith",
                "smith",
                None
            )
        ]),
        generic_selection_actions(10, false),
    )?;
    let first_progressed = settled_progressed(
        "progressed",
        "8",
        "live:11",
        11,
        10,
        "rest-select:10-smith-card-1",
        "select_card:10:smith:card:1",
        json!({"kind":"select_card","selection_id":"selection:10:smith","rest_option_id":"smith","card_id":"card:1"}),
        generic_selection_actions(11, false),
        json!([
            selector_action(
                "select_card:11:smith:card:2",
                "select_card",
                "selection:10:smith",
                "smith",
                Some("card:2")
            ),
            selector_action(
                "cancel_selection:11:smith",
                "cancel_selection",
                "selection:10:smith",
                "smith",
                None
            )
        ]),
        json!(["card:1"]),
        1,
    )?;
    let second_progressed = settled_progressed(
        "progressed",
        "10",
        "live:12",
        12,
        11,
        "rest-select:11-smith-card-2",
        "select_card:11:smith:card:2",
        json!({"kind":"select_card","selection_id":"selection:10:smith","rest_option_id":"smith","card_id":"card:2"}),
        generic_selection_actions(12, false),
        json!([
            selector_action(
                "confirm_selection:12:smith",
                "confirm_selection",
                "selection:10:smith",
                "smith",
                None
            ),
            selector_action(
                "cancel_selection:12:smith",
                "cancel_selection",
                "selection:10:smith",
                "smith",
                None
            )
        ]),
        json!(["card:1", "card:2"]),
        0,
    )?;
    let completed = settled_progressed(
        "completed",
        "12",
        "live:13",
        13,
        12,
        "rest-select:12-smith-confirm",
        "confirm_selection:12:smith",
        json!({"kind":"confirm_selection","selection_id":"selection:10:smith","rest_option_id":"smith"}),
        json!([{"action_id":"proceed:13","action":{"kind":"proceed"}}]),
        Value::Null,
        json!(["card:1", "card:2"]),
        0,
    )?;
    let mend_requested = settled_selection_requested(
        "15",
        "live:14",
        14,
        13,
        "rest-op:13:mend",
        "rest-option:13:mend",
        "mend",
        "player",
        "selection:14:mend",
        json!([{"choice_id":"player:local","label":"Ironclad","kind":"selection","domain":null}]),
        json!([
            selector_action(
                "select_player:14:mend:player:local",
                "select_player",
                "selection:14:mend",
                "mend",
                Some("player:local")
            ),
            selector_action(
                "cancel_selection:14:mend",
                "cancel_selection",
                "selection:14:mend",
                "mend",
                None
            )
        ]),
        generic_selection_actions(14, true),
    )?;
    let mut normal_responses = Vec::new();
    for (id, kind, state_id, generation, stage) in [
        (1, "state_response", "live:9", 9, "rest"),
        (2, "legal_actions_response", "live:9", 9, "rest"),
        (3, "state_response", "live:10", 10, "selection"),
        (4, "legal_actions_response", "live:10", 10, "selection"),
        (5, "legal_actions_response", "live:11", 11, "selection"),
        (6, "legal_actions_response", "live:12", 12, "selection"),
        (7, "legal_actions_response", "live:13", 13, "rest"),
        (8, "state_response", "live:14", 14, "selection"),
        (9, "legal_actions_response", "live:14", 14, "selection"),
    ] {
        normal_responses.push(rpc_value(
            id,
            v3_state(kind, &id.to_string(), state_id, generation, stage)?,
        ));
    }
    let expert_responses = vec![
        rpc_value(1, initial_rest.clone()),
        rpc_value(2, initial_rest),
        rpc_value(
            3,
            simple_response(
                "unknown",
                "3",
                "live:9",
                9,
                "rest-op:9:smith",
                "rest-option:9:smith",
                json!({"kind":"rest_option","rest_option_id":"smith"}),
            )?,
        ),
        rpc_value(4, selection_requested),
        rpc_value(
            5,
            expert_state(
                "live:10",
                10,
                "selection",
                generic_selection_actions(10, false),
            )?,
        ),
        rpc_value(
            6,
            expert_state(
                "live:10",
                10,
                "selection",
                generic_selection_actions(10, false),
            )?,
        ),
        rpc_value(
            7,
            simple_response(
                "accepted",
                "7",
                "live:10",
                10,
                "rest-select:10-smith-card-1",
                "select_card:10:smith:card:1",
                json!({"kind":"select_card","selection_id":"selection:10:smith","rest_option_id":"smith","card_id":"card:1"}),
            )?,
        ),
        rpc_value(8, first_progressed),
        rpc_value(
            9,
            expert_state(
                "live:11",
                11,
                "selection",
                generic_selection_actions(11, false),
            )?,
        ),
        rpc_value(10, second_progressed),
        rpc_value(
            11,
            expert_state(
                "live:12",
                12,
                "selection",
                generic_selection_actions(12, false),
            )?,
        ),
        rpc_value(12, completed),
        rpc_value(
            13,
            expert_state(
                "live:13",
                13,
                "rest",
                json!([rest_option("rest-option:13:mend", "mend")]),
            )?,
        ),
        rpc_value(
            14,
            simple_response(
                "unknown",
                "14",
                "live:13",
                13,
                "rest-op:13:mend",
                "rest-option:13:mend",
                json!({"kind":"rest_option","rest_option_id":"mend"}),
            )?,
        ),
        rpc_value(15, mend_requested),
        rpc_value(
            16,
            expert_state(
                "live:14",
                14,
                "selection",
                generic_selection_actions(14, true),
            )?,
        ),
        rpc_value(
            17,
            expert_state(
                "live:14",
                14,
                "selection",
                generic_selection_actions(14, true),
            )?,
        ),
        rpc_value(
            18,
            simple_response(
                "cancelled",
                "18",
                "live:14",
                14,
                "rest-cancel-14-mend",
                "cancel_selection:14:mend",
                json!({"kind":"cancel_selection","selection_id":"selection:14:mend","rest_option_id":"mend"}),
            )?,
        ),
    ];
    Ok((normal_responses, expert_responses))
}
