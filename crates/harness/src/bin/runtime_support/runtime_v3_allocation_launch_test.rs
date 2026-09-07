// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sts2_harness::{EpisodeRuntimePort, ShutdownPort};

use super::{RuntimeConfig, RuntimeV3Port, TelemetryHandle};

const INSTANCE_ID: &str = "00000000-0000-4000-8000-000000000002";
const STATIC_LEASE: &str = "00000000-0000-4000-8000-000000000099";
const ACQUIRED_LEASE: &str = "00000000-0000-4000-8000-000000000006";

struct Fixture {
    root: PathBuf,
    mcp: PathBuf,
    lease_record: PathBuf,
}

impl Fixture {
    fn new() -> Result<Self, String> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("fixture clock is before the epoch: {error}"))?
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "sts2-runtime-v3-allocation-{}-{timestamp}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&root).map_err(|error| format!("cannot create fixture: {error}"))?;
        let mcp = root.join("mcp");
        let lease_record = root.join("lease");
        let path = lease_record
            .to_str()
            .ok_or_else(|| String::from("fixture path is not UTF-8"))?;
        if path.contains(['\'', '"']) {
            return Err(String::from("fixture path cannot be quoted safely"));
        }
        let script = format!(
            "#!/bin/sh\nprintf '%s|%s\\n' \"$STS2_LEASE_ID\" \"$STS2_LEASE_EPOCH\" > \"{path}\"\n{MCP_SCRIPT}"
        );
        fs::write(&mcp, script).map_err(|error| format!("cannot write MCP fixture: {error}"))?;
        fs::set_permissions(&mcp, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("cannot make MCP fixture executable: {error}"))?;
        Ok(Self {
            root,
            mcp,
            lease_record,
        })
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _cleanup = fs::remove_dir_all(&self.root);
    }
}

const MCP_SCRIPT: &str = r#"while IFS= read -r request; do
  case "$request" in
    *'"method":"initialize"'*)
      printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{}}'
      ;;
    *'"method":"tools/list"'*)
      printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"revision":"runtime-v3-gameplay-mcp","tools":[{"name":"sts2.observe"},{"name":"sts2.legal_actions"},{"name":"sts2.dispatch_action"},{"name":"sts2.wait_for_transition"},{"name":"sts2.reobserve"},{"name":"sts2.recover"}]}}'
      ;;
    *)
      exit 1
      ;;
  esac
done
"#;

fn config(address: String, mcp: &Fixture) -> RuntimeConfig {
    RuntimeConfig {
        gateway_address: address,
        gateway_token: String::from("synthetic-token"),
        mcp_binary: mcp.mcp.display().to_string(),
        runtime_profile: String::from("runtime-v3-gameplay"),
        instance_id: String::from(INSTANCE_ID),
        caller_id: String::from("harness"),
        session_id: String::from("session-1"),
        lease_id: String::from(STATIC_LEASE),
        lease_epoch: 1,
        mcp_session_id: String::from("mcp-session-1"),
        run_id: String::from("run-1"),
        episode_id: String::from("episode-1"),
        trajectory_id: String::from("trajectory-1"),
        trace_id: String::from("trace-1"),
        artifact_id: String::from("artifact-1"),
        wait_for_combat_seconds: 0,
        settlement_timeout_seconds: 30,
        recovery_environment: Vec::new(),
    }
}

fn allocation() -> Result<Value, String> {
    let authority: Value = serde_json::from_str(include_str!(
        "../../../../../contract-artifact/runtime-allocation-v1/fixtures/valid/recovery-authority.json"
    ))
    .map_err(|error| format!("source authority fixture is invalid: {error}"))?;
    Ok(json!({
        "status": "allocated",
        "instance_id": INSTANCE_ID,
        "caller_id": "harness",
        "session_id": "session-1",
        "lease_id": ACQUIRED_LEASE,
        "lease_epoch": 3,
        "transport": "attached-loopback",
        "recovery_authority": authority
    }))
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
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                return Err(String::from("fake gateway did not receive a request"));
            }
            Err(error) => return Err(format!("fake gateway accept failed: {error}")),
        }
    }
}

fn request(stream: &mut TcpStream) -> Result<(String, Vec<u8>), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|error| format!("fake gateway read timeout setup failed: {error}"))?;
    let mut header = Vec::new();
    let mut byte = [0_u8; 1];
    while !header.ends_with(b"\r\n\r\n") {
        stream
            .read_exact(&mut byte)
            .map_err(|error| format!("fake gateway request header failed: {error}"))?;
        header.push(byte[0]);
        if header.len() > 8 * 1024 {
            return Err(String::from(
                "fake gateway request header exceeded its bound",
            ));
        }
    }
    let text =
        String::from_utf8(header).map_err(|error| format!("request was not UTF-8: {error}"))?;
    let length = text
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then_some(value.trim().parse::<usize>().ok()?)
        })
        .ok_or_else(|| String::from("fake gateway request omitted content length"))?;
    if length > 16 * 1024 {
        return Err(String::from("fake gateway request body exceeded its bound"));
    }
    let mut body = vec![0_u8; length];
    stream
        .read_exact(&mut body)
        .map_err(|error| format!("fake gateway request body failed: {error}"))?;
    Ok((text, body))
}

fn response(stream: &mut TcpStream, body: &Value) -> Result<(), String> {
    let body =
        serde_json::to_vec(body).map_err(|error| format!("response encoding failed: {error}"))?;
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .and_then(|_| stream.write_all(&body))
    .map_err(|error| format!("fake gateway response failed: {error}"))
}

fn gateway(listener: TcpListener, allocation: Value) -> Result<(), String> {
    let mut allocate = accept(&listener)?;
    let (headers, body) = request(&mut allocate)?;
    if !headers.starts_with("POST /v1/sessions/allocate ")
        || !headers.contains("x-mcp-session-id: mcp-session-1\r\n")
    {
        return Err(String::from("allocation request used the wrong route"));
    }
    let allocation_request = serde_json::from_slice::<Value>(&body)
        .map_err(|error| format!("allocation request was not JSON: {error}"))?;
    for (key, expected) in [
        ("instance_id", INSTANCE_ID),
        ("caller_id", "harness"),
        ("session_id", "session-1"),
    ] {
        if allocation_request[key].as_str() != Some(expected) {
            return Err(format!("allocation request had the wrong {key}"));
        }
    }
    response(&mut allocate, &allocation)?;

    let mut release = accept(&listener)?;
    let (headers, _) = request(&mut release)?;
    if !headers.starts_with("POST /v1/instances/00000000-0000-4000-8000-000000000002/release ")
        || !headers.contains("x-sts2-instance-id: 00000000-0000-4000-8000-000000000002\r\n")
        || !headers.contains("x-sts2-caller-id: harness\r\n")
        || !headers.contains("x-sts2-session-id: session-1\r\n")
        || !headers.contains("x-mcp-session-id: mcp-session-1\r\n")
        || !headers.contains("x-sts2-lease-id: 00000000-0000-4000-8000-000000000006\r\n")
        || !headers.contains("x-sts2-lease-epoch: 3\r\n")
        || !headers.contains("x-sts2-correlation-id: release-0001\r\n")
    {
        return Err(String::from(
            "release did not use the acquired identity and lease fence",
        ));
    }
    response(&mut release, &json!({"status": "released"}))
}

#[test]
fn launch_forwards_acquired_lease_to_mcp_and_release() -> Result<(), String> {
    let fixture = Fixture::new()?;
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("cannot bind fake gateway: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("cannot configure fake gateway: {error}"))?;
    let address = listener
        .local_addr()
        .map_err(|error| format!("cannot read fake gateway address: {error}"))?
        .to_string();
    let mut port =
        RuntimeV3Port::new_with_telemetry(config(address, &fixture), TelemetryHandle::disabled())?;
    let allocation = allocation()?;
    std::thread::scope(|scope| -> Result<(), String> {
        let gateway = scope.spawn(move || gateway(listener, allocation));
        port.launch().map_err(|error| error.to_string())?;
        if port.config.lease_id != ACQUIRED_LEASE || port.config.lease_epoch != 3 {
            return Err(String::from("launch did not install the acquired lease"));
        }
        if port.recovery_authority.is_none() {
            return Err(String::from("launch did not retain recovery authority"));
        }
        let lease = fs::read_to_string(&fixture.lease_record)
            .map_err(|error| format!("cannot read MCP lease record: {error}"))?;
        if lease.trim() != format!("{ACQUIRED_LEASE}|3") {
            return Err(format!("MCP received the wrong lease fence: {lease:?}"));
        }
        port.close_mcp()
            .map_err(|error| format!("MCP close failed: {error:?}"))?;
        port.release_lease()
            .map_err(|error| format!("lease release failed: {error:?}"))?;
        gateway
            .join()
            .map_err(|_| String::from("fake gateway panicked"))??;
        Ok(())
    })
}

#[test]
fn invalid_authority_is_rejected_without_installing_recovery_authority() -> Result<(), String> {
    let fixture = Fixture::new()?;
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("cannot bind fake gateway: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("cannot configure fake gateway: {error}"))?;
    let address = listener
        .local_addr()
        .map_err(|error| format!("cannot read fake gateway address: {error}"))?
        .to_string();
    let mut port =
        RuntimeV3Port::new_with_telemetry(config(address, &fixture), TelemetryHandle::disabled())?;
    let mut allocation = allocation()?;
    allocation["recovery_authority"]["schema_digest"] = json!("0".repeat(64));
    std::thread::scope(|scope| -> Result<(), String> {
        let gateway = scope.spawn(move || gateway(listener, allocation));
        let error = port
            .launch()
            .err()
            .ok_or_else(|| String::from("invalid authority must reject launch"))?;
        if error.code() != "gateway_allocate_invalid" {
            return Err(format!(
                "invalid authority used the wrong error code: {error}"
            ));
        }
        if port.recovery_authority.is_some() || !port.released {
            return Err(String::from(
                "invalid authority was installed or not cleaned up",
            ));
        }
        gateway
            .join()
            .map_err(|_| String::from("fake gateway panicked"))??;
        Ok(())
    })
}
