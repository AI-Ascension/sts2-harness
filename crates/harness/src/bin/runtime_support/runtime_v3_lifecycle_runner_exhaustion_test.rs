// SPDX-License-Identifier: MIT

#[test]
fn expert_observation_forward_generation_race_exhausts_bounded_reobserve()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let normal_initial = v3_state(
        "state_response",
        "1",
        "live:7",
        7,
        "combat",
        json!([{"action_id":"end:7", "action":{"kind":"end_turn"}}]),
    )?;
    let normal_refresh_one = v3_state(
        "reobserve_response",
        "2",
        "live:8",
        8,
        "combat",
        json!([{"action_id":"end:8", "action":{"kind":"end_turn"}}]),
    )?;
    let normal_refresh_two = v3_state(
        "reobserve_response",
        "3",
        "live:9",
        9,
        "combat",
        json!([{"action_id":"end:9", "action":{"kind":"end_turn"}}]),
    )?;
    let expert_refresh_one = expert_state_with_generation("live:8", 8)?;
    let expert_refresh_two = expert_state_with_generation("live:9", 9)?;
    let expert_refresh_three = expert_state_with_generation("live:10", 10)?;
    let normal_log = fixture.0.join("normal.requests");
    let expert_log = fixture.0.join("expert.requests");
    let script = format!(
        "cd '{}' || exit 1\nif [ \"$STS2_RUNTIME_PROFILE\" = \"runtime-v4-expert\" ]; then\n{}{}{}{}{}\nelse\n{}{}{}{}{}\nfi\n",
        fixture.0.display(),
        init_sequence(true),
        logged_reply_artifact(&expert_log.display().to_string(), 1, expert_refresh_one),
        logged_reply_artifact(&expert_log.display().to_string(), 2, expert_refresh_two),
        logged_reply_artifact(&expert_log.display().to_string(), 3, expert_refresh_three),
        log_requests(&expert_log.display().to_string()),
        init_sequence(false),
        logged_reply_artifact(&normal_log.display().to_string(), 1, normal_initial),
        logged_reply_artifact(&normal_log.display().to_string(), 2, normal_refresh_one),
        logged_reply_artifact(&normal_log.display().to_string(), 3, normal_refresh_two),
        log_requests(&normal_log.display().to_string())
    );
    let mut runtime_config = config("127.0.0.1:15525".into());
    runtime_config.runtime_profile = String::from("runtime-v4-expert");
    runtime_config.mcp_binary = fixture.script(&script)?;
    let mut port = RuntimeV3Port::new_with_telemetry(
        runtime_config,
        TelemetryHandle::disabled(),
    )?;
    port.allocated = true;
    let mut normal = McpProcess::spawn_profile(&port.config, "runtime-v3-gameplay")?;
    wire::initialize_mcp_profile(&mut normal, "runtime-v3-gameplay")?;
    port.mcp = Some(normal);
    let mut expert = McpProcess::spawn_profile(&port.config, "runtime-v4-expert")?;
    wire::initialize_mcp_profile(&mut expert, "runtime-v4-expert")?;
    port.expert_mcp = Some(expert);

    let error = match port.observe() {
        Ok(_) => return Err("repeated forward expert drift was accepted".into()),
        Err(error) => error,
    };
    assert_eq!(error.code(), "expert_observe_invalid");
    assert!(!error.is_retryable());
    assert!(
        error
            .message()
            .contains("advanced during bounded Runtime-v3 reobserve")
    );

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
        ["sts2.observe", "sts2.reobserve", "sts2.reobserve"]
    );
    assert_eq!(
        expert_requests
            .iter()
            .filter_map(|request| request["params"]["name"].as_str())
            .collect::<Vec<_>>(),
        ["sts2.expert_state", "sts2.expert_state", "sts2.expert_state"]
    );
    port.mcp.as_mut().ok_or("normal MCP missing")?.close()?;
    port.expert_mcp
        .as_mut()
        .ok_or("expert MCP missing")?
        .close()?;
    Ok(())
}
