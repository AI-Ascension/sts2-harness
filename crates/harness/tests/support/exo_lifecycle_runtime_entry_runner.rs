// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use std::io::Read;
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use super::fixture::Fixture;
use super::peers;

const PHASE_TIMING_ENV: &str = "STS2_OFFLINE_ENTRY_PHASE_TIMING";
static RUNNER_PHASE_EPOCH: OnceLock<Instant> = OnceLock::new();

fn phase_timing_enabled() -> bool {
    matches!(std::env::var(PHASE_TIMING_ENV).as_deref(), Ok("1"))
}

fn trace_runner_phase(phase: &'static str) {
    if !phase_timing_enabled() {
        return;
    }
    let elapsed_ms = RUNNER_PHASE_EPOCH
        .get_or_init(Instant::now)
        .elapsed()
        .as_millis();
    eprintln!(
        "offline_entry_phase_timing pid={} phase={} elapsed_ms={}",
        std::process::id(),
        phase,
        elapsed_ms
    );
}

fn emit_child_phase_timings(stderr: &str) {
    for line in stderr.lines() {
        let mut fields = line.split_whitespace();
        if fields.next() != Some("offline_entry_phase_timing") {
            continue;
        }
        let Some(pid) = fields
            .next()
            .and_then(|field| field.strip_prefix("pid="))
            .and_then(|value| value.parse::<u32>().ok())
        else {
            continue;
        };
        let Some(phase) = fields.next().and_then(|field| field.strip_prefix("phase=")) else {
            continue;
        };
        if !matches!(
            phase,
            "profile_load_start"
                | "node_verify_start"
                | "node_verify_end"
                | "exo_source_verify_end"
                | "settings_admission_begin"
                | "settings_admission_end"
                | "lifecycle_build_start"
                | "lifecycle_config_load_start"
                | "lifecycle_config_load_end"
                | "lifecycle_identity_start"
                | "lifecycle_identity_end"
                | "lifecycle_encrypted_policy_start"
                | "lifecycle_encrypted_policy_end"
                | "lifecycle_journal_owner_start"
                | "lifecycle_journal_owner_end"
                | "lifecycle_build_end"
        ) {
            continue;
        }
        let Some(elapsed_ms) = fields
            .next()
            .and_then(|field| field.strip_prefix("elapsed_ms="))
            .and_then(|value| value.parse::<u128>().ok())
        else {
            continue;
        };
        if fields.next().is_none() {
            eprintln!("offline_entry_phase_timing pid={pid} phase={phase} elapsed_ms={elapsed_ms}");
        }
    }
}

pub fn run_offline_lifecycle_entry() -> Result<(), String> {
    trace_runner_phase("runner_start");
    let mut fixture = Fixture::new()?;
    fixture.prepare_policy()?;
    trace_runner_phase("fixture_complete");
    let gateway = fixture
        .gateway
        .take()
        .ok_or_else(|| String::from("offline gateway listener was already moved"))?;
    let gateway_worker = std::thread::spawn(move || {
        trace_runner_phase("gateway_worker_start");
        peers::serve_gateway(gateway)
    });
    let mut command = fixture.command()?;
    if phase_timing_enabled() {
        command.env(PHASE_TIMING_ENV, "1");
    }
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot launch shipped runtime: {error}"))?;
    let deadline = Instant::now() + Duration::from_secs(30);
    trace_runner_phase("child_spawn");
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("cannot poll shipped runtime: {error}"))?
        {
            break status;
        }
        if Instant::now() >= deadline {
            trace_runner_phase("child_timeout");
            let _ = child.kill();
            let _ = child.wait();
            return Err(String::from(
                "shipped runtime exceeded the offline fixture deadline",
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    trace_runner_phase("child_end");
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
    if phase_timing_enabled() {
        emit_child_phase_timings(&stderr);
    }
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
