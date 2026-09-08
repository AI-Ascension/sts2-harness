// SPDX-License-Identifier: MIT

use super::McpProcess;
use serde_json::json;
use std::fs;
use std::time::{Duration, Instant};
use sts2_harness::ExecutionCancellation;
use tokio::process::Command;

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn cancelled_execution_never_launches_primary_or_recovery_mcp() {
    let cancellation = ExecutionCancellation::default();
    cancellation.cancel();
    let config = super::tests::session_config();
    assert!(
        matches!(McpProcess::spawn_with_cancellation(&config, &cancellation), Err(error) if error == "MCP launch cancelled")
    );
    assert!(
        matches!(McpProcess::spawn_recovery(&config, "instance", "lease", 1, &json!({}), &cancellation), Err(error) if error == "MCP recovery launch cancelled")
    );
}

#[test]
fn cancellation_after_request_write_poisons_session_and_reaps_child() -> TestResult {
    cancel_exchange(
        "read request; printf ready >\"$1\"; exec /bin/sleep 30",
        json!({}),
    )
}

#[test]
fn cancellation_interrupts_a_full_mcp_input_pipe() -> TestResult {
    cancel_exchange(
        "printf ready >\"$1\"; exec /bin/sleep 30",
        json!({"data": "x".repeat(128 * 1024)}),
    )
}

fn cancel_exchange(script: &str, parameters: serde_json::Value) -> TestResult {
    let marker = std::env::temp_dir().join(format!("mcp-cancel-{}", uuid::Uuid::new_v4()));
    let mut command = Command::new("/bin/sh");
    command.args(["-c", script, "fixture"]).arg(&marker);
    let mut process = McpProcess::spawn_command(command, Duration::from_secs(30))?;
    let cancellation = process.cancellation.clone();
    let (result, observed, started) = std::thread::scope(|scope| {
        let watcher = scope.spawn(|| {
            let deadline = Instant::now() + Duration::from_secs(3);
            while !marker.exists() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            let observed = marker.exists();
            let started = Instant::now();
            cancellation.cancel();
            (observed, started)
        });
        let result = process.call(1, "tools/call", parameters);
        let (observed, started) = watcher.join().map_err(|_| "cancellation watcher failed")?;
        Ok::<_, Box<dyn std::error::Error>>((result, observed, started))
    })?;
    if marker.exists() {
        fs::remove_file(&marker)?;
    }
    assert!(
        observed,
        "synthetic MCP did not confirm startup/request receipt"
    );
    assert!(
        matches!(result, Err(error) if error == "MCP exchange cancelled; outcome remains uncertain")
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(process.is_closed());
    assert_eq!(process.child.id(), None);
    assert!(process.call(2, "tools/call", json!({})).is_err());
    process.close()?;
    Ok(())
}
