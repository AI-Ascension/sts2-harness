// SPDX-License-Identifier: MIT

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sts2_harness::{ExecutionFingerprint, ExecutionLineage, ExecutionStore, ExecutionStoreConfig};

use super::super::durable::DurableHandle;
use super::reconnect_support::*;
use super::*;

const B_DEPLOYMENT_ID: &str = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const B_INSTANCE_ID: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
const B_INCARCATION: &str = "dddddddd-dddd-4ddd-8ddd-dddddddddddd";
const B_BOOT_ID: &str = "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee";
const B_LEASE_ID: &str = "ffffffff-ffff-4fff-8fff-ffffffffffff";
const B_FENCE_ID: &str = "12121212-1212-4121-8121-121212121212";

fn fresh_allocation_authority(
    mut authority: super::allocation_context::RecoveryAuthority,
) -> super::allocation_context::RecoveryAuthority {
    authority.boot_id = B_BOOT_ID.to_owned();
    authority.authority_generation = 2;
    authority.lease_id = B_LEASE_ID.to_owned();
    authority.lease_epoch = 2;
    authority.current_fence = json!({
        "host_fence_id": B_FENCE_ID,
        "deployment_id": authority.deployment_id.clone(),
        "instance_id": authority.instance_id.clone(),
        "instance_incarnation": authority.instance_incarnation.clone(),
        "boot_id": B_BOOT_ID,
        "authority_generation": 2,
        "fence_generation": 2,
        "created_at": "2026-09-07T00:00:00Z"
    });
    authority
}

fn authority_b() -> super::allocation_context::RecoveryAuthority {
    let mut authority = recovery_authority();
    authority.deployment_id = B_DEPLOYMENT_ID.to_owned();
    authority.instance_id = B_INSTANCE_ID.to_owned();
    authority.instance_incarnation = B_INCARCATION.to_owned();
    fresh_allocation_authority(authority)
}

fn same_host_fresh_authority() -> super::allocation_context::RecoveryAuthority {
    fresh_allocation_authority(recovery_authority())
}

fn authority_value(authority: &super::allocation_context::RecoveryAuthority) -> Value {
    json!({
        "contract": "watchdog-runtime-allocation-v1",
        "schema_digest": super::allocation_context::ALLOCATION_SCHEMA_DIGEST,
        "context": {
            "deployment_id": authority.deployment_id,
            "instance_id": authority.instance_id.clone(),
            "instance_incarnation": authority.instance_incarnation,
            "boot_id": authority.boot_id,
            "authority_generation": authority.authority_generation,
            "lease_id": authority.lease_id.clone(),
            "lease_epoch": authority.lease_epoch
        },
        "current_fence": authority.current_fence
    })
}

fn request(stream: &mut TcpStream) -> Result<(String, Vec<u8>), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|error| format!("gateway read timeout setup failed: {error}"))?;
    let mut header = Vec::new();
    let mut byte = [0_u8; 1];
    while !header.ends_with(b"\r\n\r\n") {
        stream
            .read_exact(&mut byte)
            .map_err(|error| format!("gateway request header failed: {error}"))?;
        header.push(byte[0]);
        if header.len() > 8 * 1024 {
            return Err(String::from("gateway request header exceeded its bound"));
        }
    }
    let text =
        String::from_utf8(header).map_err(|error| format!("gateway header UTF-8: {error}"))?;
    let length = text
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then_some(value.trim().parse::<usize>().ok()?)
        })
        .ok_or_else(|| String::from("gateway request omitted content length"))?;
    if length > 16 * 1024 {
        return Err(String::from("gateway request body exceeded its bound"));
    }
    let mut body = vec![0_u8; length];
    stream
        .read_exact(&mut body)
        .map_err(|error| format!("gateway request body failed: {error}"))?;
    Ok((text, body))
}

fn accept(listener: &TcpListener) -> Result<TcpStream, String> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Ok(stream),
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                std::thread::yield_now();
            }
            Err(error) => return Err(format!("gateway accept failed: {error}")),
        }
    }
}

fn respond(stream: &mut TcpStream, body: &Value) -> Result<(), String> {
    let body =
        serde_json::to_vec(body).map_err(|error| format!("gateway response encoding: {error}"))?;
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .and_then(|_| stream.write_all(&body))
    .map_err(|error| format!("gateway response failed: {error}"))
}

fn gateway(
    listener: TcpListener,
    authority: &super::allocation_context::RecoveryAuthority,
) -> Result<(), String> {
    let mut allocation = accept(&listener)?;
    let (headers, body) = request(&mut allocation)?;
    if !headers.starts_with("POST /v1/sessions/allocate ")
        || !headers.contains("x-mcp-session-id: mcp-session-b\r\n")
    {
        return Err(String::from("allocation did not use the fresh session"));
    }
    let allocation_request: Value = serde_json::from_slice(&body)
        .map_err(|error| format!("allocation request was not JSON: {error}"))?;
    if allocation_request["instance_id"].as_str() != Some(authority.instance_id.as_str())
        || allocation_request["caller_id"] != "harness"
        || allocation_request["session_id"] != "session-b"
    {
        return Err(String::from(
            "allocation request used the wrong current identity",
        ));
    }
    respond(
        &mut allocation,
        &json!({
            "status": "allocated",
            "instance_id": authority.instance_id,
            "caller_id": "harness",
            "session_id": "session-b",
            "lease_id": authority.lease_id,
            "lease_epoch": authority.lease_epoch,
            "transport": "attached-loopback",
            "recovery_authority": authority_value(authority)
        }),
    )?;

    let mut release = accept(&listener)?;
    let (headers, _) = request(&mut release)?;
    let expected = [
        format!("POST /v1/instances/{}/release ", authority.instance_id),
        format!("x-sts2-instance-id: {}\r\n", authority.instance_id),
        String::from("x-sts2-session-id: session-b\r\n"),
        String::from("x-mcp-session-id: mcp-session-b\r\n"),
        format!("x-sts2-lease-id: {}\r\n", authority.lease_id),
        format!("x-sts2-lease-epoch: {}\r\n", authority.lease_epoch),
    ];
    for expected in expected {
        if !headers.contains(&expected) {
            return Err(format!(
                "release omitted fresh allocation field {expected:?}"
            ));
        }
    }
    respond(&mut release, &json!({"status": "released"}))
}

fn recovery_script_with_environment(
    fixture: &Fixture,
    lookup: &Value,
    reconcile: &Value,
    authority: &super::allocation_context::RecoveryAuthority,
) -> Result<String, Box<dyn std::error::Error>> {
    let path = response_script_with_identity(
        fixture,
        lookup,
        reconcile,
        &authority.instance_id,
        "session-b",
        &authority.lease_id,
        authority.lease_epoch,
    )?;
    let script = fs::read_to_string(&path)?;
    let body = script
        .strip_prefix("#!/bin/sh\n")
        .ok_or("response fixture omitted its shell header")?;
    let body = body.replace("|| exit 1", "|| exit 0");
    let env_record = fixture.0.join("env-record");
    fs::write(
        &path,
        format!(
            "#!/bin/sh\nprintf '%s|%s|%s|%s\\n' \"$STS2_RUNTIME_PROFILE\" \"$STS2_RECOVERY_TOKEN\" \"$STS2_RECOVERY_PRINCIPAL_ID\" \"$STS2_RECOVERY_CURRENT_FENCE_JSON\" >> '{}'\n{body}",
            env_record.display()
        ),
    )?;
    Ok(path)
}

#[test]
fn disk_reopen_new_incarnation_preserves_historical_context_but_blocks_continuation()
-> Result<(), Box<dyn std::error::Error>> {
    run_disk_reopen(authority_b(), false)
}

#[test]
fn disk_reopen_same_host_fresh_authority_continues_with_a_fresh_witness()
-> Result<(), Box<dyn std::error::Error>> {
    run_disk_reopen(same_host_fresh_authority(), true)
}

fn run_disk_reopen(
    authority: super::allocation_context::RecoveryAuthority,
    continue_after_recovery: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let database = fixture.0.join("execution.sqlite3");
    let lineage = ExecutionLineage::new("run-1", "episode-1", "attempt-a", "trajectory-a")?;
    let fingerprint = ExecutionFingerprint::new("seed", "build", "state", "config", "provider")?;
    let mut store = ExecutionStore::open(ExecutionStoreConfig::new(&database))?;
    store.start_episode(&lineage, &fingerprint)?;
    let durable_a =
        DurableHandle::from_store_for_test(store, lineage.clone(), fingerprint.clone())?;
    let mut config_a = config("127.0.0.1:15525".into());
    config_a.mcp_binary = dispatch_script(&fixture)?;
    let mut port_a =
        RuntimeV3Port::new_with_store(config_a, TelemetryHandle::disabled(), durable_a.clone())?;
    port_a.recovery_authority = Some(recovery_authority());
    let action = sts2_harness::EpisodeLegalAction::new(
        "combat.end-turn-pending",
        sts2_harness::ActionKind::EndTurn,
    )?;
    durable_a.operation_intent(
        PENDING_OPERATION_ID,
        PENDING_STATE_ID,
        0,
        &action,
        &json!({"kind":"end_turn"}),
        &json!({"state_id":PENDING_STATE_ID,"generation":0,"legal_actions":[
            {"action_id":"combat.end-turn-pending","action":{"kind":"end_turn"}}
        ]}),
    )?;
    let payload_digest = durable_a.operation_payload_digest(PENDING_OPERATION_ID)?;
    durable_a.operation_dispatched(PENDING_OPERATION_ID, &payload_digest)?;
    let historical = durable_a
        .pending_operations()?
        .pop()
        .ok_or("missing historical operation")?;
    let encoded = encode_base64(
        historical
            .intent
            .action_payload
            .as_deref()
            .ok_or("missing action")?,
    );
    let catalog_digest = historical
        .intent
        .catalog_digest
        .clone()
        .ok_or("missing catalog digest")?;
    let (lookup, reconcile) = settled_frames(
        PENDING_OPERATION_ID,
        PENDING_STATE_ID,
        0,
        &payload_digest,
        &catalog_digest,
        encoded.trim_end_matches('='),
    );
    drop(port_a);
    durable_a.close()?;
    drop(durable_a);

    let reopened = ExecutionStore::open(ExecutionStoreConfig::new(&database))?;
    let durable_b = DurableHandle::from_store_for_test(reopened, lineage, fingerprint)?;
    let mut runtime_config = config("127.0.0.1:0".into());
    runtime_config.instance_id = authority.instance_id.clone();
    runtime_config.session_id = String::from("session-b");
    runtime_config.mcp_session_id = String::from("mcp-session-b");
    runtime_config.lease_id = authority.lease_id.clone();
    runtime_config.lease_epoch = authority.lease_epoch;
    runtime_config.recovery_environment = RecoveryEnvironment::new()
        .0
        .into_iter()
        .map(|(name, value)| match name.as_str() {
            "STS2_RECOVERY_TOKEN" => (name, String::from("recovery-token-b")),
            "STS2_RECOVERY_PRINCIPAL_ID" => (name, String::from("principal-b")),
            _ => (name, value),
        })
        .collect();
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    runtime_config.gateway_address = listener.local_addr()?.to_string();
    runtime_config.mcp_binary =
        recovery_script_with_environment(&fixture, &lookup, &reconcile, &authority)?;
    let mut port_b = RuntimeV3Port::new_with_store(
        runtime_config,
        TelemetryHandle::disabled(),
        durable_b.clone(),
    )?;
    let gateway_authority = authority.clone();
    std::thread::scope(|scope| -> Result<(), Box<dyn std::error::Error>> {
        let gateway = scope.spawn(move || gateway(listener, &gateway_authority));
        if continue_after_recovery {
            port_b.launch().map_err(|error| error.to_string())?;
            port_b
                .close_mcp()
                .map_err(|error| format!("MCP close failed: {error:?}"))?;
            port_b
                .release_lease()
                .map_err(|error| format!("lease release failed: {error:?}"))?;
        } else {
            let error = port_b
                .launch()
                .err()
                .ok_or("replacement incarnation unexpectedly continued")?;
            assert_eq!(error.code(), "runtime_resume_failed");
            assert!(!error.is_retryable());
            assert!(port_b.released, "failed resume did not release its lease");
        }
        gateway.join().map_err(|_| "gateway thread panicked")??;
        Ok(())
    })?;
    let state = durable_b.operation_state(PENDING_OPERATION_ID)?;
    if continue_after_recovery {
        assert_eq!(state, sts2_harness::OperationState::Reconciled);
    } else {
        assert!(
            state.is_unresolved(),
            "replacement operation was closed: {state:?}"
        );
    }
    let requests = fs::read_to_string(fixture.0.join("requests"))?;
    let calls: Vec<Value> = requests
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    let recovery_calls: Vec<&Value> = calls
        .iter()
        .filter(|call| {
            call["params"]["name"]
                .as_str()
                .is_some_and(|name| name.starts_with("watchdog."))
        })
        .collect();
    assert_eq!(
        recovery_calls
            .iter()
            .map(|call| call["params"]["name"].as_str())
            .collect::<Vec<_>>(),
        vec![
            Some("watchdog.operation_lookup"),
            Some("watchdog.operation_reconcile")
        ]
    );
    assert!(
        !calls
            .iter()
            .any(|call| call["params"]["name"] == "sts2.dispatch_action")
    );
    let wait_calls: Vec<&Value> = calls
        .iter()
        .filter(|call| call["params"]["name"] == "sts2.wait_for_transition")
        .collect();
    assert_eq!(wait_calls.len(), usize::from(continue_after_recovery));
    if let Some(wait) = wait_calls.first() {
        assert_eq!(
            wait["params"]["arguments"]["wait_for_millis"],
            json!(1),
            "continuation must use the bounded retained-witness read"
        );
        assert_eq!(
            wait["params"]["arguments"]["instance_id"],
            authority.instance_id
        );
        assert_eq!(wait["params"]["arguments"]["lease_id"], authority.lease_id);
        assert_eq!(
            wait["params"]["arguments"]["lease_epoch"],
            authority.lease_epoch
        );
        assert_eq!(
            wait["params"]["arguments"]["operation_id"],
            PENDING_OPERATION_ID
        );
    }
    let lookup_operation = &recovery_calls[0]["params"]["arguments"]["payload"]["operation"];
    assert_eq!(
        lookup_operation["original_context"]["boot_id"],
        "66666666-6666-4666-8666-666666666666"
    );
    let reconcile_payload = &recovery_calls[1]["params"]["arguments"]["payload"];
    assert_eq!(reconcile_payload["operation"], *lookup_operation);
    assert_eq!(reconcile_payload["current_fence"], authority.current_fence);
    let environments = fs::read_to_string(fixture.0.join("env-record"))?;
    assert!(environments.contains("watchdog-recovery-v1|recovery-token-b|principal-b|"));
    assert!(environments.contains(&authority.current_fence.to_string()));
    Ok(())
}
