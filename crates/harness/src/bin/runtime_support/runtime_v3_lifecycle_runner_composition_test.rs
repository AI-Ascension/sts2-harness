// SPDX-License-Identifier: MIT

fn expert_state_with_generation(
    state_id: &str,
    generation: u64,
) -> Result<Value, Box<dyn std::error::Error>> {
    let mut value = expert_state(state_id, generation, "combat", false)?;
    for action in value["legal_actions"]
        .as_array_mut()
        .ok_or("expert fixture omitted legal actions")?
    {
        let action_id = action["action_id"]
            .as_str()
            .ok_or("expert fixture action omitted action ID")?
            .replace(":7", &format!(":{generation}"));
        action["action_id"] = json!(action_id);
    }
    Ok(value)
}

fn logged_reply_artifact(log: &str, id: u64, value: Value) -> String {
    let response = json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {"content": [{"type": "text", "text": value.to_string()}]}
    });
    format!(
        "IFS= read -r line || exit 1\nprintf '%s\\n' \"$line\" >> '{log}'\nprintf '%s\\n' '{}'\n",
        response.to_string().replace('\'', "'\\''")
    )
}

fn log_requests(log: &str) -> String {
    format!(
        "while IFS= read -r line; do printf '%s\\n' \"$line\" >> '{}'; done\n",
        log
    )
}

#[test]
fn expert_observation_reobserves_after_forward_generation_race()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let initial_actions = json!([
        {"action_id":"end:7", "action":{"kind":"end_turn"}}
    ]);
    let refreshed_actions = json!([
        {"action_id":"end:8", "action":{"kind":"end_turn"}}
    ]);
    let normal_initial = v3_state(
        "state_response",
        "1",
        "live:7",
        7,
        "combat",
        initial_actions,
    )?;
    let normal_refresh = v3_state(
        "reobserve_response",
        "2",
        "live:8",
        8,
        "combat",
        refreshed_actions,
    )?;
    let expert_refresh = expert_state_with_generation("live:8", 8)?;
    let normal_log = fixture.0.join("normal.requests");
    let expert_log = fixture.0.join("expert.requests");
    let script = format!(
        "cd '{}' || exit 1\nif [ \"$STS2_RUNTIME_PROFILE\" = \"runtime-v4-expert\" ]; then\n{}{}{}{}else\n{}{}{}{}fi\n",
        fixture.0.display(),
        init_sequence(true),
        logged_reply_artifact(&expert_log.display().to_string(), 1, expert_refresh.clone()),
        logged_reply_artifact(&expert_log.display().to_string(), 2, expert_refresh),
        log_requests(&expert_log.display().to_string()),
        init_sequence(false),
        logged_reply_artifact(&normal_log.display().to_string(), 1, normal_initial),
        logged_reply_artifact(&normal_log.display().to_string(), 2, normal_refresh),
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

    let observation = port.observe()?;
    assert_eq!(observation.state_id(), "live:8");
    assert_eq!(observation.generation(), 8);
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
        ["sts2.observe", "sts2.reobserve"]
    );
    assert_eq!(
        expert_requests
            .iter()
            .filter_map(|request| request["params"]["name"].as_str())
            .collect::<Vec<_>>(),
        ["sts2.expert_state", "sts2.expert_state"]
    );
    port.mcp.as_mut().ok_or("normal MCP missing")?.close()?;
    port.expert_mcp
        .as_mut()
        .ok_or("expert MCP missing")?
        .close()?;
    Ok(())
}

include!("runtime_v3_lifecycle_runner_exhaustion_test.rs");

#[test]
fn expert_catalog_forward_generation_race_enters_bounded_reobserve()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let initial_actions = json!([
        {"action_id":"end:7", "action":{"kind":"end_turn"}}
    ]);
    let refreshed_actions = json!([
        {"action_id":"end:8", "action":{"kind":"end_turn"}}
    ]);
    let normal_initial = v3_state(
        "state_response",
        "1",
        "live:7",
        7,
        "combat",
        initial_actions.clone(),
    )?;
    let mut initial_catalog = v3_state(
        "legal_actions_response",
        "2",
        "live:7",
        7,
        "combat",
        initial_actions,
    )?;
    initial_catalog["observation"] = Value::Null;
    let normal_refresh = v3_state(
        "reobserve_response",
        "3",
        "live:8",
        8,
        "combat",
        refreshed_actions.clone(),
    )?;
    let mut refreshed_catalog = v3_state(
        "legal_actions_response",
        "4",
        "live:8",
        8,
        "combat",
        refreshed_actions,
    )?;
    refreshed_catalog["observation"] = Value::Null;
    let expert_initial = expert_state_with_generation("live:7", 7)?;
    let expert_refresh = expert_state_with_generation("live:8", 8)?;
    let normal_log = fixture.0.join("normal.requests");
    let expert_log = fixture.0.join("expert.requests");
    let script = format!(
        "cd '{}' || exit 1\nif [ \"$STS2_RUNTIME_PROFILE\" = \"runtime-v4-expert\" ]; then\n{}{}{}{}{}{}else\n{}{}{}{}{}{}fi\n",
        fixture.0.display(),
        init_sequence(true),
        logged_reply_artifact(&expert_log.display().to_string(), 1, expert_initial),
        logged_reply_artifact(&expert_log.display().to_string(), 2, expert_refresh.clone()),
        logged_reply_artifact(&expert_log.display().to_string(), 3, expert_refresh.clone()),
        logged_reply_artifact(&expert_log.display().to_string(), 4, expert_refresh),
        log_requests(&expert_log.display().to_string()),
        init_sequence(false),
        logged_reply_artifact(&normal_log.display().to_string(), 1, normal_initial),
        logged_reply_artifact(&normal_log.display().to_string(), 2, initial_catalog),
        logged_reply_artifact(&normal_log.display().to_string(), 3, normal_refresh),
        logged_reply_artifact(&normal_log.display().to_string(), 4, refreshed_catalog),
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

    let observation = port.observe()?;
    assert_eq!(observation.state_id(), "live:7");
    let error = match port.legal_actions("live:7", 7) {
        Ok(_) => return Err("forward expert catalog drift was accepted".into()),
        Err(error) => error,
    };
    assert_eq!(error.code(), "catalog_reobserve");
    assert!(error.is_retryable());

    let fresh = sts2_harness::RecoveryPort::reobserve(&mut port)?;
    assert_eq!(fresh.state_id(), "live:8");
    assert_eq!(fresh.generation(), 8);
    let actions = port.legal_actions("live:8", 8)?;
    assert_eq!(actions.state_id(), "live:8");
    assert_eq!(actions.generation(), 8);

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
        [
            "sts2.observe",
            "sts2.legal_actions",
            "sts2.reobserve",
            "sts2.legal_actions"
        ]
    );
    assert_eq!(
        expert_requests
            .iter()
            .filter_map(|request| request["params"]["name"].as_str())
            .collect::<Vec<_>>(),
        [
            "sts2.expert_state",
            "sts2.expert_state",
            "sts2.expert_state",
            "sts2.expert_state"
        ]
    );
    port.mcp.as_mut().ok_or("normal MCP missing")?.close()?;
    port.expert_mcp
        .as_mut()
        .ok_or("expert MCP missing")?
        .close()?;
    Ok(())
}

#[test]
fn expert_observation_retrograde_and_wrong_identity_fail_without_reobserve()
-> Result<(), Box<dyn std::error::Error>> {
    for (expert_state_id, expert_generation) in [("live:6", 6), ("other:7", 7)] {
        let fixture = Fixture::new()?;
        let normal_initial = v3_state(
            "state_response",
            "1",
            "live:7",
            7,
            "combat",
            json!([{"action_id":"end:7", "action":{"kind":"end_turn"}}]),
        )?;
        let expert_mismatch = expert_state_with_generation(expert_state_id, expert_generation)?;
        let normal_log = fixture.0.join("normal.requests");
        let expert_log = fixture.0.join("expert.requests");
        let script = format!(
            "cd '{}' || exit 1\nif [ \"$STS2_RUNTIME_PROFILE\" = \"runtime-v4-expert\" ]; then\n{}{}{}else\n{}{}{}fi\n",
            fixture.0.display(),
            init_sequence(true),
            logged_reply_artifact(&expert_log.display().to_string(), 1, expert_mismatch),
            log_requests(&expert_log.display().to_string()),
            init_sequence(false),
            logged_reply_artifact(&normal_log.display().to_string(), 1, normal_initial),
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
            Ok(_) => return Err("invalid expert identity was accepted".into()),
            Err(error) => error,
        };
        assert!(error.message().contains("does not match"));
        port.mcp.as_mut().ok_or("normal MCP missing")?.close()?;
        port.expert_mcp
            .as_mut()
            .ok_or("expert MCP missing")?
            .close()?;

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
            ["sts2.observe"]
        );
        assert_eq!(
            expert_requests
                .iter()
                .filter_map(|request| request["params"]["name"].as_str())
                .collect::<Vec<_>>(),
            ["sts2.expert_state"]
        );
    }
    Ok(())
}
