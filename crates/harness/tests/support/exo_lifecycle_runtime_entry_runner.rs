// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use std::io::Read;
use std::process::Stdio;
use std::time::{Duration, Instant};

use super::fixture::Fixture;
use super::peers;

pub fn run_offline_lifecycle_entry() -> Result<(), String> {
    let mut fixture = Fixture::new()?;
    fixture.prepare_policy()?;
    let gateway = fixture
        .gateway
        .take()
        .ok_or_else(|| String::from("offline gateway listener was already moved"))?;
    let gateway_worker = std::thread::spawn(move || peers::serve_gateway(gateway));
    let mut child = fixture
        .command()?
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot launch shipped runtime: {error}"))?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("cannot poll shipped runtime: {error}"))?
        {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(String::from(
                "shipped runtime exceeded the offline fixture deadline",
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    child
        .stdout
        .take()
        .ok_or("runtime stdout was not captured")?
        .read_to_string(&mut stdout)
        .map_err(|error| error.to_string())?;
    child
        .stderr
        .take()
        .ok_or("runtime stderr was not captured")?
        .read_to_string(&mut stderr)
        .map_err(|error| error.to_string())?;
    if !status.success() {
        let gateway_result = gateway_worker
            .join()
            .map_err(|_| String::from("offline gateway panicked"));
        let peer_log = std::fs::read_to_string(fixture.action_log.with_extension("peer.log"))
            .unwrap_or_default();
        return Err(format!(
            "shipped runtime failed ({status}): stdout={stdout:?}; stderr={stderr:?}; gateway={gateway_result:?}; mcp={peer_log:?}; address={}",
            fixture.gateway_address,
        ));
    }
    gateway_worker
        .join()
        .map_err(|_| String::from("offline gateway panicked"))??;
    if !stdout.contains(r#""status":"complete""#)
        || !stdout.contains(r#""terminal_stage":"victory""#)
    {
        return Err(format!(
            "shipped runtime did not complete the fixture episode: {stdout:?}"
        ));
    }
    let action: Value = serde_json::from_slice(
        &std::fs::read(&fixture.action_log)
            .map_err(|error| format!("runtime did not dispatch the observed action: {error}"))?,
    )
    .map_err(|error| format!("dispatched action log was invalid: {error}"))?;
    if action != json!({"action_id":"combat.end-turn","action":{"kind":"end_turn"}}) {
        return Err(format!("runtime dispatched an unexpected action: {action}"));
    }
    let mcp_log = std::fs::read_to_string(fixture.action_log.with_extension("peer.log"))
        .map_err(|error| format!("runtime MCP exchange log is missing: {error}"))?;
    let calls: Vec<_> = mcp_log
        .lines()
        .filter(|line| line.starts_with("tools/call "))
        .collect();
    if calls
        != [
            "tools/call sts2.observe",
            "tools/call sts2.legal_actions",
            "tools/call sts2.dispatch_action",
            "tools/call sts2.wait_for_transition",
        ]
    {
        return Err(format!(
            "runtime used an unexpected MCP exchange sequence: {calls:?}"
        ));
    }
    if !fixture.effect_log.exists() {
        return Err(String::from(
            "the shipped runtime did not cross the lifecycle provider send boundary",
        ));
    }
    let effects =
        std::fs::read_to_string(&fixture.effect_log).map_err(|error| error.to_string())?;
    if effects.lines().collect::<Vec<_>>() != ["provider-effect"] {
        return Err(format!(
            "the shipped runtime did not issue exactly one lifecycle provider effect: {effects:?}"
        ));
    }
    super::receipt::assert_persisted_receipt(&fixture)?;
    Ok(())
}
