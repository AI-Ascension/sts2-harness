// SPDX-License-Identifier: MIT

#[test]
fn idle_transition_retries_a_lagging_expert_snapshot_before_accepting_successor()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let initial = v3_state(
        "state_response",
        "1",
        "live:7",
        7,
        "combat",
        json!([{"action_id":"end:7", "action":{"kind":"end_turn"}}]),
    )?;
    let successor = v3_state(
        "state_response",
        "2",
        "live:8",
        8,
        "combat",
        json!([{"action_id":"end:8", "action":{"kind":"end_turn"}}]),
    )?;
    let expert_lagging = expert_state_with_generation("live:6", 6)?;
    let expert_initial = expert_state_with_generation("live:7", 7)?;
    let expert_successor = expert_state_with_generation("live:8", 8)?;
    let normal_log = fixture.0.join("normal.requests");
    let expert_log = fixture.0.join("expert.requests");
    let script = format!(
        "cd '{}' || exit 1\nif [ \"$STS2_RUNTIME_PROFILE\" = \"runtime-v4-expert\" ]; then\n{}{}{}{}{}\nelse\n{}{}{}{}\nfi\n",
        fixture.0.display(),
        init_sequence(true),
        logged_reply_artifact(&expert_log.display().to_string(), 1, expert_lagging),
        logged_reply_artifact(&expert_log.display().to_string(), 2, expert_initial),
        logged_reply_artifact(&expert_log.display().to_string(), 3, expert_successor),
        log_requests(&expert_log.display().to_string()),
        init_sequence(false),
        logged_reply_artifact(&normal_log.display().to_string(), 1, initial),
        logged_reply_artifact(&normal_log.display().to_string(), 2, successor),
        log_requests(&normal_log.display().to_string()),
    );
    let mut runtime_config = config("127.0.0.1:15525".into());
    runtime_config.runtime_profile = String::from("runtime-v4-expert");
    runtime_config.mcp_binary = fixture.script(&script)?;
    let mut port = RuntimeV3Port::new_with_telemetry(
        runtime_config,
        TelemetryHandle::disabled(),
    )?;
    port.allocated = true;
    // The idle operation is observing the state after an already-settled action. Keep its
    // identity bound to generation seven while the first normal read races with an older expert
    // projection.
    port.generation = 7;
    let mut normal = McpProcess::spawn_profile(&port.config, "runtime-v3-gameplay")?;
    wire::initialize_mcp_profile(&mut normal, "runtime-v3-gameplay")?;
    port.mcp = Some(normal);
    let mut expert = McpProcess::spawn_profile(&port.config, "runtime-v4-expert")?;
    wire::initialize_mcp_profile(&mut expert, "runtime-v4-expert")?;
    port.expert_mcp = Some(expert);

    let sample = sts2_harness::BarrierPort::wait_for_transition(
        &mut port,
        "episode-idle-7",
        500,
    )?;
    assert_eq!(sample.outcome(), sts2_harness::WaitOutcome::Successor);
    assert_eq!(sample.observation().ok_or("missing successor")?.generation(), 8);
    assert_eq!(port.generation, 8);

    let normal_requests: Vec<Value> = fs::read_to_string(normal_log)?
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    let expert_requests: Vec<Value> = fs::read_to_string(expert_log)?
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    assert_eq!(
        normal_requests
            .iter()
            .filter_map(|request| request["params"]["name"].as_str())
            .collect::<Vec<_>>(),
        ["sts2.observe", "sts2.observe"]
    );
    assert_eq!(
        expert_requests
            .iter()
            .filter_map(|request| request["params"]["name"].as_str())
            .collect::<Vec<_>>(),
        ["sts2.expert_state", "sts2.expert_state", "sts2.expert_state"]
    );
    assert!(normal_requests
        .iter()
        .chain(expert_requests.iter())
        .all(|request| request["params"]["name"] != "sts2.dispatch_action"));
    port.mcp.as_mut().ok_or("normal MCP missing")?.close()?;
    port.expert_mcp
        .as_mut()
        .ok_or("expert MCP missing")?
        .close()?;
    Ok(())
}
