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
        trace_id: "trace-1".into(),
        artifact_id: "artifact-1".into(),
        wait_for_combat_seconds: 0,
        settlement_timeout_seconds: 30,
    }
}

fn accept(listener: &TcpListener) -> Result<TcpStream, String> {
    accept_until(listener, Duration::from_secs(3))
}

fn accept_until(listener: &TcpListener, timeout: Duration) -> Result<TcpStream, String> {
    let deadline = Instant::now() + timeout;
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
    let mut port = RuntimeV3Port::new_with_telemetry(
        config(listener.local_addr()?.to_string()),
        TelemetryHandle::disabled(),
    )?;
    std::thread::scope(|scope| -> Result<(), Box<dyn std::error::Error>> {
        let gateway = scope.spawn(move || -> Result<(), String> {
            let mut allocation = accept(&listener)?;
            let headers = request(&mut allocation).map_err(|error| error.to_string())?;
            assert!(headers.starts_with("POST /v1/sessions/allocate "));
            assert!(headers.contains("x-mcp-session-id: mcp-session-1\r\n"));
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

fn wrong_lease_gateway(listener: TcpListener, status: &str) -> Result<(), String> {
    let mut allocation = accept(&listener)?;
    let headers = request(&mut allocation).map_err(|error| error.to_string())?;
    assert!(headers.contains("x-mcp-session-id: mcp-session-explicit\r\n"));
    let body = json!({
        "status":"allocated", "instance_id":"instance-1", "caller_id":"harness",
        "session_id":"session-1", "lease_id":"returned-lease", "lease_epoch":9
    })
    .to_string();
    write!(
        allocation,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .map_err(|error| error.to_string())?;
    drop(allocation);
    let mut release = accept(&listener)?;
    let headers = request(&mut release).map_err(|error| error.to_string())?;
    assert!(headers.starts_with("POST /v1/instances/instance-1/release "));
    assert!(headers.contains("x-sts2-lease-id: returned-lease\r\n"));
    assert!(headers.contains("x-sts2-lease-epoch: 9\r\n"));
    let body = json!({"status":status}).to_string();
    write!(
        release,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

#[test]
fn runtime_v3_wrong_lease_uses_returned_fence_and_requires_release_confirmation()
-> Result<(), Box<dyn std::error::Error>> {
    for status in ["released", "rejected"] {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let mut config = config(listener.local_addr()?.to_string());
        config.mcp_session_id = "mcp-session-explicit".into();
        let mut port = RuntimeV3Port::new_with_telemetry(config, TelemetryHandle::disabled())?;
        std::thread::scope(|scope| -> Result<(), Box<dyn std::error::Error>> {
            let gateway = scope.spawn(move || wrong_lease_gateway(listener, status));
            let error = port
                .launch()
                .err()
                .ok_or("wrong lease must reject launch")?;
            assert_eq!(error.code(), "gateway_allocate_invalid");
            assert!(port.allocated);
            assert_eq!(port.released, status == "released");
            assert!(port.mcp.is_none());
            if status == "rejected" {
                assert!(
                    error
                        .to_string()
                        .contains("cleanup did not confirm release")
                );
            }
            gateway.join().map_err(|_| "fake gateway panicked")??;
            Ok(())
        })?;
    }
    Ok(())
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

    mod runner {
        include!("runtime_v3_lifecycle_runner_test.rs");
    }

    fn reply(value: Value) -> String {
        format!(
            "IFS= read -r line || exit 1\nprintf '%s\\n' \"$line\" >> requests\nprintf '%s\\n' '{}'\n",
            value.to_string().replace('\'', "'\\''")
        )
    }

    mod recovery {
        include!("runtime_v3_lifecycle_recovery_test.rs");
    }
}
