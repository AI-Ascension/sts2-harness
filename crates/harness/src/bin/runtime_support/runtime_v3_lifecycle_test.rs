// SPDX-License-Identifier: MIT

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use sts2_harness::EpisodeRuntimePort;

use super::*;

fn config(address: String) -> RuntimeConfig {
    RuntimeConfig {
        gateway_address: address,
        gateway_token: "synthetic-token".into(),
        mcp_binary: "unused-test-binary".into(),
        runtime_profile: "runtime-v3-gameplay".into(),
        instance_id: "instance-1".into(),
        caller_id: "harness".into(),
        session_id: "session-1".into(),
        mcp_session_id: "mcp-session-1".into(),
        lease_id: "lease-1".into(),
        lease_epoch: 1,
        run_id: "run-1".into(),
        episode_id: "episode-1".into(),
        trajectory_id: "trajectory-1".into(),
        artifact_id: "artifact-1".into(),
        wait_for_combat_seconds: 0,
        settlement_timeout_seconds: 0,
    }
}

fn accept(listener: &TcpListener) -> Result<TcpStream, String> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Ok(stream),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err("fake gateway did not receive expected cleanup".into());
                }
                std::thread::yield_now();
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

fn request(stream: &mut TcpStream) -> Result<String, Box<dyn std::error::Error>> {
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    let mut bytes = Vec::new();
    let mut byte = [0_u8; 1];
    while !bytes.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte)?;
        bytes.push(byte[0]);
        if bytes.len() > 8192 {
            return Err("oversized synthetic request".into());
        }
    }
    let headers = String::from_utf8(bytes)?;
    let length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then_some(value.trim())
        })
        .ok_or("missing length")?
        .parse::<usize>()?;
    if length > 16384 {
        return Err("oversized synthetic body".into());
    }
    let mut body = vec![0; length];
    stream.read_exact(&mut body)?;
    Ok(headers)
}

#[test]
fn runtime_v3_lost_allocation_response_releases_the_configured_lease()
-> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let mut port = RuntimeV3Port::new(config(listener.local_addr()?.to_string()))?;
    std::thread::scope(|scope| -> Result<(), Box<dyn std::error::Error>> {
        let gateway = scope.spawn(move || -> Result<(), String> {
            let mut allocation = accept(&listener)?;
            let headers = request(&mut allocation).map_err(|error| error.to_string())?;
            assert!(headers.starts_with("POST /v1/sessions/allocate "));
            // Allocation has committed; its response is lost before the client reads it.
            drop(allocation);
            let mut release = accept(&listener)?;
            let headers = request(&mut release).map_err(|error| error.to_string())?;
            assert!(headers.starts_with("POST /v1/instances/instance-1/release "));
            for expected in [
                "x-sts2-lease-id: lease-1",
                "x-sts2-lease-epoch: 1",
                "x-sts2-session-id: session-1",
            ] {
                assert!(headers.contains(expected));
            }
            let body = r#"{"status":"released"}"#;
            write!(
                release,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .map_err(|error| error.to_string())?;
            Ok(())
        });
        assert!(port.launch().is_err());
        assert!(port.allocated && port.released);
        gateway.join().map_err(|_| "fake gateway panicked")??;
        Ok(())
    })
}

#[cfg(unix)]
mod reconnect {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use sts2_harness::{ActionIdentity, RecoveryPort};

    use super::*;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Result<Self, std::io::Error> {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "sts2-v3-reconnect-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path)?;
            Ok(Self(path))
        }

        fn script(&self, content: &str) -> Result<String, Box<dyn std::error::Error>> {
            let path = self.0.join("mcp");
            fs::write(&path, format!("#!/bin/sh\n{content}"))?;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
            Ok(path.to_str().ok_or("non-UTF8 fixture path")?.to_owned())
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _cleanup = fs::remove_dir_all(&self.0);
        }
    }

    fn reply(value: Value) -> String {
        format!(
            "IFS= read -r line || exit 1\nprintf '%s\\n' \"$line\" >> requests\nprintf '%s\\n' '{}'\n",
            value.to_string().replace('\'', "'\\''")
        )
    }

    fn recovery_script(fixture: &Fixture) -> Result<(), Box<dyn std::error::Error>> {
        let mut settled: Value = serde_json::from_str(include_str!(
            "../../../../../protocol-artifact/runtime-v3-gameplay/golden/dispatch-action-settled.json"
        ))?;
        settled["kind"] = json!("recover_response");
        settled["correlation_id"] = json!("2");
        let tools: Vec<_> = [
            "sts2.observe",
            "sts2.legal_actions",
            "sts2.dispatch_action",
            "sts2.wait_for_transition",
            "sts2.reobserve",
            "sts2.recover",
        ]
        .into_iter()
        .map(|name| json!({"name":name}))
        .collect();
        let script = format!(
            "cd '{}' || exit 1\n{}{}{}",
            fixture.0.display(),
            reply(json!({"jsonrpc":"2.0","id":1,"result":{}})),
            reply(
                json!({"jsonrpc":"2.0","id":2,"result":{"revision":"runtime-v3-gameplay-mcp","tools":tools}})
            ),
            reply(
                json!({"jsonrpc":"2.0","id":2,"result":{"content":[{"text":settled.to_string()}]}})
            )
        );
        fixture.script(&script)?;
        Ok(())
    }

    #[test]
    fn runtime_v3_reconnect_reconciles_same_operation_without_redispatch()
    -> Result<(), Box<dyn std::error::Error>> {
        let fixture = Fixture::new()?;
        let mut config = config("127.0.0.1:15525".into());
        config.mcp_binary = fixture.script("IFS= read -r line\nexit 0\n")?;
        let mut port = RuntimeV3Port::new(config)?;
        port.allocated = true;
        port.mcp = Some(McpProcess::spawn(&port.config)?);
        let mut state: Value = serde_json::from_str(include_str!(
            "../../../../../protocol-artifact/runtime-v3-gameplay/golden/state-response.json"
        ))?;
        state["legal_actions"] = json!([
            {"action_id":"combat.end-turn", "action":{"kind":"end_turn"}}
        ]);
        let parsed = parse::observation(&state, "state_response", &port.config)?;
        let action = parsed.actions.actions()[0].clone();
        let observation = port.install(parsed);
        let identity = ActionIdentity::new(
            "op-1",
            observation.state_id(),
            observation.generation(),
            action.action_id(),
        )?;
        assert!(port.dispatch_action(&identity, &action).is_err());
        assert!(port.mcp.as_ref().is_some_and(McpProcess::is_closed));
        recovery_script(&fixture)?;
        let receipt = port.reconcile("op-1")?;
        assert_eq!(receipt.operation_id(), "op-1");
        assert_eq!(receipt.status(), sts2_harness::DispatchStatus::Settled);
        assert_eq!(port.operations.len(), 1);
        assert_eq!(port.reconnect_attempts, 1);
        let requests = fs::read_to_string(fixture.0.join("requests"))?;
        let requests: Vec<Value> = requests
            .lines()
            .map(serde_json::from_str)
            .collect::<Result<_, _>>()?;
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[2]["params"]["name"], "sts2.recover");
        assert_eq!(requests[2]["params"]["arguments"]["operation_id"], "op-1");
        assert!(
            !requests
                .iter()
                .any(|value| value["params"]["name"] == "sts2.dispatch_action")
        );
        port.mcp.as_mut().ok_or("missing MCP")?.close()?;
        port.reconnect_attempts = 2;
        assert!(port.reconcile("op-1").is_err());
        Ok(())
    }
}
