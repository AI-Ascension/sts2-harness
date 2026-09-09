// SPDX-License-Identifier: MIT

fn action(
    actions: &sts2_harness::EpisodeLegalActionSet,
    id: &str,
) -> Result<sts2_harness::EpisodeLegalAction, Box<dyn std::error::Error>> {
    actions
        .find(id)
        .cloned()
        .ok_or_else(|| format!("missing action {id}").into())
}

fn identity(
    operation_id: &str,
    state_id: &str,
    generation: u64,
    action: &sts2_harness::EpisodeLegalAction,
) -> Result<ActionIdentity, Box<dyn std::error::Error>> {
    Ok(ActionIdentity::new(
        operation_id,
        state_id,
        generation,
        action.action_id(),
    )?)
}

#[test]
fn rest_action_transport_reconciles_selectors_and_preserves_operation_identity()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let script = fixture.script(&fake_mcp_script(&fixture)?)?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let mut runtime_config = super::config(listener.local_addr()?.to_string());
    runtime_config.mcp_binary = script;
    runtime_config.runtime_profile = String::from("runtime-v4-expert-rest-action");
    let mut port = RuntimeV3Port::new_with_telemetry(runtime_config, TelemetryHandle::disabled())?;
    let gateway = std::thread::spawn(move || fake_gateway(listener));

    port.launch()?;
    let observation = port.observe()?;
    assert_eq!(observation.state_id(), "live:9");
    assert_eq!(observation.generation(), 9);
    assert_eq!(observation.stage(), sts2_harness::EpisodeStage::Rest);
    let legal = port.legal_actions(observation.state_id(), observation.generation())?;
    let smith = action(&legal, "rest-option:9:smith")?;
    let smith_identity = identity("rest-op:9:smith", "live:9", 9, &smith)?;
    let unknown = port.dispatch_action(&smith_identity, &smith)?;
    assert_eq!(unknown.status(), DispatchStatus::Unknown);
    let requested = port.reconcile("rest-op:9:smith")?;
    assert_eq!(requested.status(), DispatchStatus::Settled);
    assert_eq!(requested.after().map(|after| after.generation()), Some(10));

    let observed_selection = port.observe()?;
    assert_eq!(observed_selection.state_id(), "live:10");
    let selection_legal = port.legal_actions(
        observed_selection.state_id(),
        observed_selection.generation(),
    )?;
    let card_one = action(&selection_legal, "select_card:10:smith:card:1")?;
    let card_one_identity = identity("rest-select:10-smith-card-1", "live:10", 10, &card_one)?;
    let accepted = port.dispatch_action(&card_one_identity, &card_one)?;
    assert_eq!(accepted.status(), DispatchStatus::Accepted);
    let progressed = port.reconcile("rest-select:10-smith-card-1")?;
    assert_eq!(progressed.status(), DispatchStatus::Settled);
    assert_eq!(progressed.after().map(|after| after.generation()), Some(11));

    let selection_legal = port.legal_actions("live:11", 11)?;
    let card_two = action(&selection_legal, "select_card:11:smith:card:2")?;
    let card_two_identity = identity("rest-select:11-smith-card-2", "live:11", 11, &card_two)?;
    let progressed = port.dispatch_action(&card_two_identity, &card_two)?;
    assert_eq!(progressed.status(), DispatchStatus::Settled);
    assert_eq!(progressed.after().map(|after| after.generation()), Some(12));

    let selection_legal = port.legal_actions("live:12", 12)?;
    let confirm = action(&selection_legal, "confirm_selection:12:smith")?;
    let confirm_identity = identity("rest-select:12-smith-confirm", "live:12", 12, &confirm)?;
    let completed = port.dispatch_action(&confirm_identity, &confirm)?;
    assert_eq!(completed.status(), DispatchStatus::Settled);
    assert_eq!(completed.after().map(|after| after.generation()), Some(13));

    let mend_legal = port.legal_actions("live:13", 13)?;
    let mend = action(&mend_legal, "rest-option:13:mend")?;
    let mend_identity = identity("rest-op:13:mend", "live:13", 13, &mend)?;
    let mend_unknown = port.dispatch_action(&mend_identity, &mend)?;
    assert_eq!(mend_unknown.status(), DispatchStatus::Unknown);
    let mend_requested = port.reconcile("rest-op:13:mend")?;
    assert_eq!(mend_requested.status(), DispatchStatus::Settled);
    assert_eq!(
        mend_requested.after().map(|after| after.generation()),
        Some(14)
    );

    let observed_mend = port.observe()?;
    let mend_selection =
        port.legal_actions(observed_mend.state_id(), observed_mend.generation())?;
    let cancel = action(&mend_selection, "cancel_selection:14:mend")?;
    let cancel_identity = identity("rest-cancel-14-mend", "live:14", 14, &cancel)?;
    let cancelled = port.dispatch_action(&cancel_identity, &cancel)?;
    assert_eq!(cancelled.status(), DispatchStatus::Cancelled);
    assert_eq!(cancelled.operation_id(), "rest-cancel-14-mend");

    EpisodeShutdown.close(&mut port)?;
    gateway.join().map_err(|_| "fake gateway panicked")??;

    let expert_requests = fixture.read_requests("expert.requests")?;
    let names = expert_requests[2..]
        .iter()
        .map(|request| request["params"]["name"].as_str().unwrap_or(""))
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "sts2.expert_state",
            "sts2.expert_state",
            "sts2.expert_rest_action",
            "sts2.expert_rest_reconcile",
            "sts2.expert_state",
            "sts2.expert_state",
            "sts2.expert_rest_action",
            "sts2.expert_rest_reconcile",
            "sts2.expert_state",
            "sts2.expert_rest_action",
            "sts2.expert_state",
            "sts2.expert_rest_action",
            "sts2.expert_state",
            "sts2.expert_rest_action",
            "sts2.expert_rest_reconcile",
            "sts2.expert_state",
            "sts2.expert_state",
            "sts2.expert_rest_action"
        ]
    );
    let action_calls = [2_usize, 6, 9, 11, 13, 17];
    assert_eq!(
        action_calls
            .iter()
            .map(|index| names[*index])
            .collect::<Vec<_>>(),
        [
            "sts2.expert_rest_action",
            "sts2.expert_rest_action",
            "sts2.expert_rest_action",
            "sts2.expert_rest_action",
            "sts2.expert_rest_action",
            "sts2.expert_rest_action"
        ]
    );
    assert_eq!(
        expert_requests[4]["params"]["arguments"]["operation_id"],
        "rest-op:9:smith"
    );
    assert_eq!(
        expert_requests[5]["params"]["arguments"]["operation_id"],
        "rest-op:9:smith"
    );
    assert_eq!(
        expert_requests[4]["params"]["arguments"]["action"]["action"]["rest_option_id"],
        "smith"
    );
    assert_eq!(
        expert_requests[19]["params"]["arguments"]["action"]["action"]["selection_id"],
        "selection:14:mend"
    );
    let normal_requests = fixture.read_requests("normal.requests")?;
    assert_eq!(normal_requests.len(), 11);
    assert!(normal_requests[2..].iter().all(|request| {
        !request["params"]["name"]
            .as_str()
            .unwrap_or("")
            .starts_with("sts2.expert_")
    }));
    Ok(())
}
