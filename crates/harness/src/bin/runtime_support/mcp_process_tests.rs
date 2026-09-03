// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn child_environment_preserves_distinct_gateway_and_mcp_sessions() {
    let config = RuntimeConfig {
        gateway_address: "127.0.0.1:15525".into(),
        gateway_token: "synthetic-token".into(),
        mcp_binary: "unused-test-binary".into(),
        instance_id: "instance-1".into(),
        caller_id: "harness".into(),
        session_id: "gateway-session-1".into(),
        mcp_session_id: "mcp-session-1".into(),
        lease_id: "lease-1".into(),
        lease_epoch: 1,
        runtime_profile: "runtime-v1".into(),
        run_id: "run-1".into(),
        episode_id: "episode-1".into(),
        trajectory_id: "trajectory-1".into(),
        artifact_id: "artifact-1".into(),
        wait_for_combat_seconds: 0,
        settlement_timeout_seconds: 30,
        runtime_v3_card_index: 0,
        runtime_v3_target_id: None,
    };
    let command = McpProcess::configured_command(&config);
    let environment: std::collections::BTreeMap<_, _> = command.as_std().get_envs().collect();
    for (name, expected) in [
        ("STS2_SESSION_ID", "gateway-session-1"),
        ("STS2_MCP_SESSION_ID", "mcp-session-1"),
        ("STS2_RUNTIME_PROFILE", "runtime-v1"),
    ] {
        assert_eq!(
            environment.get(std::ffi::OsStr::new(name)),
            Some(&Some(std::ffi::OsStr::new(expected)))
        );
    }
}

#[test]
fn response_requires_exact_envelope_identity_and_outcome() {
    for response in [
        json!({"jsonrpc":"2.0","id":2,"result":{}}),
        json!({"jsonrpc":"1.0","id":1,"result":{}}),
        json!({"jsonrpc":"2.0","id":"1","result":{}}),
        json!({"jsonrpc":"2.0","id":1}),
        json!({"jsonrpc":"2.0","id":1,"result":{},"error":{}}),
    ] {
        assert!(validate_response(response.to_string().as_bytes(), 1).is_err());
    }
    assert!(validate_response(br#"{"jsonrpc":"2.0","id":1,"result":{}}"#, 1).is_ok());
}

#[cfg(unix)]
fn shell(script: &str) -> Result<McpProcess, String> {
    let mut command = Command::new("/bin/sh");
    command.args(["-c", script]);
    McpProcess::spawn_command(command, Duration::from_millis(75))
}

#[test]
#[cfg(unix)]
fn nonreading_child_write_is_in_deadline_and_process_is_reaped() -> Result<(), String> {
    let mut process = shell("exec /bin/sleep 3")?;
    let start = Instant::now();
    let result = process.call(1, "test", json!({"payload":"x".repeat(60 * 1024)}));
    assert_eq!(result, Err(String::from("MCP exchange timed out")));
    assert!(start.elapsed() < Duration::from_secs(2));
    assert!(process.child.id().is_none());
    assert!(process.input.is_none() && process.output.is_none());
    assert!(process.call(2, "test", json!({})).is_err());
    Ok(())
}

#[test]
#[cfg(unix)]
fn oversized_undelimited_response_is_rejected_without_waiting_for_exit() -> Result<(), String> {
    let mut process = shell("/usr/bin/head -c 70000 /dev/zero; exec /bin/sleep 3")?;
    process.timeout = Duration::from_secs(2);
    let start = Instant::now();
    assert_eq!(
        process.call(1, "test", json!({})),
        Err(String::from("MCP response exceeded its size limit"))
    );
    assert!(start.elapsed() < Duration::from_secs(2));
    assert!(process.child.id().is_none());
    Ok(())
}

#[test]
#[cfg(unix)]
fn slow_trickle_does_not_restart_deadline() -> Result<(), String> {
    let mut process = shell("while :; do printf x; /bin/sleep 0.02; done")?;
    let start = Instant::now();
    assert_eq!(
        process.call(1, "test", json!({})),
        Err(String::from("MCP exchange timed out"))
    );
    assert!(start.elapsed() < Duration::from_secs(2));
    assert!(process.child.id().is_none());
    Ok(())
}

#[test]
#[cfg(unix)]
fn inherited_descriptors_cannot_strand_a_worker() -> Result<(), String> {
    let mut process = shell("read first; /bin/sleep 2 & exit 0")?;
    let start = Instant::now();
    assert_eq!(
        process.call(1, "test", json!({})),
        Err(String::from("MCP exchange timed out"))
    );
    assert!(start.elapsed() < Duration::from_secs(1));
    assert!(process.input.is_none() && process.output.is_none());
    Ok(())
}

#[test]
#[cfg(unix)]
fn shutdown_and_drop_are_bounded_for_stalled_children() -> Result<(), String> {
    let mut process = shell("exec /bin/sleep 3")?;
    let start = Instant::now();
    assert_eq!(process.close(), Err(String::from("MCP shutdown timed out")));
    assert!(start.elapsed() < Duration::from_secs(2));
    assert!(process.child.id().is_none());
    assert!(process.close().is_ok());
    let process = shell("exec /bin/sleep 3")?;
    let start = Instant::now();
    drop(process);
    assert!(start.elapsed() < Duration::from_secs(2));
    Ok(())
}

#[test]
#[cfg(unix)]
fn separate_calls_preserve_session_and_buffered_frames() -> Result<(), String> {
    let mut process = shell(
        "read first; printf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}'; read second; printf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{}}'",
    )?;
    process.timeout = Duration::from_secs(2);
    assert!(process.call(1, "test", json!({})).is_ok());
    assert!(process.call(2, "test", json!({})).is_ok());
    process.close()
}

#[test]
#[cfg(unix)]
fn full_duplex_does_not_deadlock_on_pipe_capacity() -> Result<(), String> {
    let mut process = shell(
        "printf '%s' '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":\"'; /usr/bin/head -c 60000 /dev/zero | /usr/bin/tr '\\000' x; printf '%s\\n' '\"}'; read first",
    )?;
    process.timeout = Duration::from_secs(2);
    let result = process.call(1, "test", json!({"payload":"x".repeat(60000)}))?;
    assert_eq!(result["result"].as_str().map(str::len), Some(60000));
    process.close()
}

#[test]
#[cfg(unix)]
fn per_call_budget_can_extend_the_default_for_a_semantic_wait() -> Result<(), String> {
    let mut process = shell(
        "read first; /bin/sleep 0.15; printf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}'",
    )?;
    assert!(
        process
            .call_with_timeout(1, "test", json!({}), Duration::from_secs(2))
            .is_ok()
    );
    process.close()
}

#[test]
#[cfg(unix)]
fn malformed_reply_poisons_session_without_exposing_child_output() -> Result<(), String> {
    let mut process = shell(
        "read first; printf '%s\\n' '{\"jsonrpc\":\"1.0\",\"id\":1,\"result\":\"PRIVATE_PAYLOAD\"}'; exec /bin/sleep 3",
    )?;
    process.timeout = Duration::from_secs(2);
    assert_eq!(
        process.call(1, "test", json!({})),
        Err(String::from("MCP response envelope was invalid"))
    );
    assert!(process.child.id().is_none());
    assert!(process.call(2, "test", json!({})).is_err());
    Ok(())
}

#[test]
#[cfg(unix)]
fn synchronous_process_is_safe_inside_an_async_caller() -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|_| String::from("test runtime unavailable"))?;
    runtime.block_on(async {
        let mut process =
            shell("read first; printf '%s\\n' '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}'")?;
        process.call_with_timeout(1, "test", json!({}), Duration::from_secs(2))?;
        process.close()
    })
}

#[test]
fn spawn_failure_is_safe_inside_an_async_caller() -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|_| String::from("test runtime unavailable"))?;
    runtime.block_on(async {
        let result = McpProcess::spawn_command(
            Command::new("/missing-sts2-mcp-test-binary"),
            Duration::from_secs(1),
        );
        assert_eq!(
            result.err(),
            Some(String::from("MCP process failed to start"))
        );
    });
    Ok(())
}
