// SPDX-License-Identifier: MIT
#![cfg(target_os = "linux")]

use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use sts2_harness::worker_bootstrap::BOOTSTRAP_MAGIC;

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn actual_worker_entry_consumes_only_bounded_pipe_bootstrap()
-> Result<(), Box<dyn std::error::Error>> {
    let json = include_bytes!("../../../protocol-artifact/worker-bootstrap-v1/valid/linux.json");
    let mut frame = BOOTSTRAP_MAGIC.to_vec();
    frame.extend_from_slice(&u32::try_from(json.len())?.to_be_bytes());
    frame.extend_from_slice(json);
    for case in ["valid", "not-pipe", "silent", "environment-boot"] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_sts2-harness-runtime"));
        command
            .env_clear()
            .env("STS2_GATEWAY_TOKEN", "synthetic-unused-token")
            .env("STS2_WORKER_MODE", "true")
            .stdin(if case == "not-pipe" {
                Stdio::null()
            } else {
                Stdio::piped()
            })
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        if case == "environment-boot" {
            command
                .env("STS2_RUNTIME_PROFILE", "runtime-v3-gameplay")
                .env("STS2_MCP_BINARY", "/nonexistent/synthetic-mcp")
                .env("STS2_WORKER_DEPLOYMENT_ID", "deployment-1")
                .env("STS2_WORKER_PROFILE_DIGEST", "a".repeat(64))
                .env("STS2_BUILD_DIGEST", "b".repeat(64))
                .env("STS2_RUNTIME_CONFIG_DIGEST", "c".repeat(64))
                .env(
                    "STS2_WATCHDOG_BOOT_ID",
                    "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb",
                );
        }
        let mut child = OwnedChild(command.spawn()?);
        let mut writer = child.0.stdin.take();
        if matches!(case, "valid" | "environment-boot") {
            writer
                .as_mut()
                .ok_or("missing bootstrap writer")?
                .write_all(&frame)?;
        }
        // Keep the writer open: a valid frame must complete without EOF, and
        // a silent producer must still expire the absolute startup deadline.
        let deadline = Instant::now() + Duration::from_secs(if case == "silent" { 7 } else { 2 });
        loop {
            if let Some(status) = child.0.try_wait()? {
                assert_eq!(status.code(), Some(2));
                break;
            }
            assert!(
                Instant::now() < deadline,
                "worker startup exceeded bound: {case}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut stderr = String::new();
        child
            .0
            .stderr
            .take()
            .ok_or("missing stderr")?
            .take(1024)
            .read_to_string(&mut stderr)?;
        let expected = match case {
            "valid" => "worker mode requires the runtime-v3-gameplay profile",
            "not-pipe" => "invalid worker bootstrap pipe or frame",
            "silent" => "worker bootstrap deadline expired",
            _ => "watchdog boot must come from the worker bootstrap pipe",
        };
        assert_eq!(stderr, format!("sts2-harness runtime failed: {expected}\n"));
        drop(writer);
    }
    Ok(())
}
