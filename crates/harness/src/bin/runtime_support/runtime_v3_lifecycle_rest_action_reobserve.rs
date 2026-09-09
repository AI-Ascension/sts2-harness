// SPDX-License-Identifier: MIT

#[test]
fn rest_selection_completion_reconcile_uses_operation_selector_after_reobserve()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let initial = expert_state(
        "live:9",
        9,
        "rest",
        json!([rest_option("rest-option:9:mend", "mend")]),
    )?;
    let requested = settled_selection_requested(
        "3",
        "live:10",
        10,
        9,
        "rest-op:9:mend",
        "rest-option:9:mend",
        "mend",
        "player",
        "selection:10:mend",
        json!([{"choice_id":"player:local","label":"Ironclad","kind":"selection","domain":null}]),
        json!([
            selector_action(
                "select_player:10:mend:player:local",
                "select_player",
                "selection:10:mend",
                "mend",
                Some("player:local")
            ),
            selector_action(
                "cancel_selection:10:mend",
                "cancel_selection",
                "selection:10:mend",
                "mend",
                None
            )
        ]),
        generic_selection_actions(10, true),
    )?;
    let selection_action = json!({
        "kind":"select_player",
        "selection_id":"selection:10:mend",
        "rest_option_id":"mend",
        "player_id":"player:local"
    });
    let unknown = simple_response(
        "unknown",
        "5",
        "live:10",
        10,
        "rest-select:10-mend-player",
        "select_player:10:mend:player:local",
        selection_action.clone(),
    )?;
    let fresh = expert_state(
        "live:11",
        11,
        "rest",
        json!([rest_option("rest-option:11:proceed", "proceed")]),
    )?;
    let completed = settled_mend_selection_completed(
        "7",
        "live:11",
        11,
        10,
        "rest-select:10-mend-player",
        "select_player:10:mend:player:local",
        "selection:10:mend",
        "player:local",
        json!([rest_option("rest-option:11:proceed", "proceed")]),
    )?;
    let normal_responses = vec![
        rpc_value(1, v3_state("state_response", "1", "live:9", 9, "rest")?),
        rpc_value(2, v3_state("legal_actions_response", "2", "live:9", 9, "rest")?),
        rpc_value(3, v3_state("legal_actions_response", "3", "live:10", 10, "selection")?),
        rpc_value(4, v3_state("state_response", "4", "live:11", 11, "rest")?),
    ];
    let expert_responses = vec![
        rpc_value(1, initial.clone()),
        rpc_value(2, initial),
        rpc_value(3, requested),
        rpc_value(
            4,
            expert_state(
                "live:10",
                10,
                "selection",
                generic_selection_actions(10, true),
            )?,
        ),
        rpc_value(5, unknown),
        rpc_value(6, fresh),
        rpc_value(7, completed),
    ];
    let script = fixture.script(&fake_mcp_script_with_sets(
        &fixture,
        normal_responses,
        expert_responses,
    )?)?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let mut runtime_config = super::config(listener.local_addr()?.to_string());
    runtime_config.mcp_binary = script;
    runtime_config.runtime_profile = String::from("runtime-v4-expert-rest-action");
    let mut port = RuntimeV3Port::new_with_telemetry(runtime_config, TelemetryHandle::disabled())?;
    let gateway = std::thread::spawn(move || fake_gateway(listener));

    port.launch()?;
    let observation = port.observe()?;
    let legal = port.legal_actions(observation.state_id(), observation.generation())?;
    let mend = action(&legal, "rest-option:9:mend")?;
    let mend_identity = identity("rest-op:9:mend", "live:9", 9, &mend)?;
    let requested = port.dispatch_action(&mend_identity, &mend)?;
    assert_eq!(requested.status(), DispatchStatus::Settled);

    let selection_legal = port.legal_actions("live:10", 10)?;
    let select_player = action(&selection_legal, "select_player:10:mend:player:local")?;
    let select_identity = identity(
        "rest-select:10-mend-player",
        "live:10",
        10,
        &select_player,
    )?;
    let unknown = port.dispatch_action(&select_identity, &select_player)?;
    assert_eq!(unknown.status(), DispatchStatus::Unknown);

    let fresh = port.observe()?;
    assert_eq!(fresh.state_id(), "live:11");
    assert_eq!(fresh.generation(), 11);
    let completed = port.reconcile("rest-select:10-mend-player")?;
    assert_eq!(completed.status(), DispatchStatus::Settled);
    assert_eq!(completed.after().map(|after| after.generation()), Some(11));

    EpisodeShutdown.close(&mut port)?;
    gateway.join().map_err(|_| "fake gateway panicked")??;

    let expert_requests = fixture.read_requests("expert.requests")?;
    let dispatched = expert_requests
        .iter()
        .filter(|request| request["params"]["name"] == "sts2.expert_rest_action")
        .collect::<Vec<_>>();
    assert_eq!(dispatched.len(), 2);
    assert_eq!(
        dispatched
            .iter()
            .filter(|request| {
                request["params"]["arguments"]["operation_id"]
                    == "rest-select:10-mend-player"
            })
            .count(),
        1
    );
    let reconciled = expert_requests
        .iter()
        .filter(|request| request["params"]["name"] == "sts2.expert_rest_reconcile")
        .collect::<Vec<_>>();
    assert_eq!(reconciled.len(), 1);
    assert_eq!(
        reconciled[0]["params"]["arguments"]["operation_id"],
        "rest-select:10-mend-player"
    );
    Ok(())
}
