// SPDX-License-Identifier: MIT

//! The shipped CLI must terminate a blocked SQLite admission and compose a fresh backend.
//! This uses the existing synthetic command, with no game, MCP, gateway or provider process.

#![cfg(target_os = "linux")]
#![allow(clippy::expect_used)]

use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use rusqlite::Connection;
use rustix::event::{PollFd, PollFlags, poll};
use rustix::process::{Pid, PidfdFlags, pidfd_open};
use serde_json::{Value, json};
use sts2_harness::management::{ClientResponse, ManagementClient};

const TOKEN: &str = "owned-cli-synthetic-token";

struct Process {
    pid: u32,
    descriptor: OwnedFd,
}

impl Process {
    fn child_of(parent: u32) -> Self {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            for entry in fs::read_dir("/proc").expect("procfs").flatten() {
                let Some(pid) = entry
                    .file_name()
                    .to_str()
                    .and_then(|name| name.parse::<i32>().ok())
                    .and_then(Pid::from_raw)
                else {
                    continue;
                };
                let before = identity(pid);
                if before.is_some_and(|value| value.0 == parent)
                    && let Ok(descriptor) = pidfd_open(pid, PidfdFlags::empty())
                    && identity(pid) == before
                {
                    return Self {
                        pid: pid.as_raw_nonzero().get() as u32,
                        descriptor,
                    };
                }
            }
            assert!(Instant::now() < deadline, "owned child startup deadline");
            thread::sleep(Duration::from_millis(2));
        }
    }

    fn exited(&self) -> bool {
        let mut descriptors = [PollFd::new(&self.descriptor, PollFlags::IN)];
        poll(
            &mut descriptors,
            Some(&rustix::event::Timespec {
                tv_sec: 0,
                tv_nsec: 0,
            }),
        )
        .expect("pidfd poll");
        descriptors[0].revents().contains(PollFlags::IN)
    }

    fn await_exit(&self) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.exited() {
            assert!(Instant::now() < deadline, "owned process exit deadline");
            thread::sleep(Duration::from_millis(2));
        }
    }
}

fn identity(pid: Pid) -> Option<(u32, u64)> {
    let stat = fs::read_to_string(format!("/proc/{}/stat", pid.as_raw_nonzero())).ok()?;
    let suffix = stat.get(stat.rfind(')')? + 1..)?;
    let fields: Vec<_> = suffix.split_whitespace().collect();
    Some((fields.get(1)?.parse().ok()?, fields.get(19)?.parse().ok()?))
}

struct Cli {
    child: Child,
    root: PathBuf,
    address: SocketAddr,
    client: ManagementClient,
}

impl Cli {
    fn start() -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/owned-cli-tests")
            .join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).expect("test directory");
        let reservation = TcpListener::bind("127.0.0.1:0").expect("loopback reservation");
        let address: SocketAddr = reservation.local_addr().expect("address");
        let mut command = Command::new(env!("CARGO_BIN_EXE_sts2-workflow"));
        command
            .args(["serve", "--listen", &address.to_string(), "--store"])
            .arg(root.join("workflow.sqlite"))
            .args(["--auth-profile", "owned-cli"])
            .env("STS2_WORKFLOW_TOKEN_OWNED_CLI", TOKEN)
            .env("STS2_MANAGEMENT_DISPATCH_MILLIS", "250")
            .env_remove("STS2_MANAGEMENT_OWNED_ROLE")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(
                fs::File::create(root.join("stderr")).expect("stderr file"),
            ));
        drop(reservation);
        let child = command.spawn().expect("shipped workflow command");
        let mut cli = Self {
            child,
            root,
            address,
            client: ManagementClient::new(address, TOKEN).expect("client"),
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if cli
                .client
                .request_json("GET", "/v1/health", None)
                .is_ok_and(|response| response.status == 200)
            {
                return cli;
            }
            assert!(cli.child.try_wait().expect("CLI status").is_none());
            assert!(Instant::now() < deadline, "CLI startup deadline");
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn request(&self, method: &str, path: &str, body: Option<&[u8]>) -> ClientResponse {
        self.client
            .request_json(method, path, body)
            .expect("bounded command response")
    }

    fn stop_frontend(&mut self) {
        self.child.kill().expect("stop directly owned frontend");
        self.child.wait().expect("reap directly owned frontend");
    }

    fn diagnostic_loss(&self) -> u64 {
        let mut stream = TcpStream::connect(self.address).expect("health peer");
        stream
            .set_read_timeout(Some(Duration::from_secs(4)))
            .expect("read bound");
        stream
            .write_all(b"GET /v1/health HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .expect("health request");
        let mut response = String::new();
        stream
            .take(65_536)
            .read_to_string(&mut response)
            .expect("health response");
        assert!(response.starts_with("HTTP/1.1 200"));
        response
            .lines()
            .find_map(|line| line.strip_prefix("X-Ascension-Diagnostic-Loss: "))
            .expect("diagnostic loss header")
            .trim()
            .parse()
            .expect("loss count")
    }
}

impl Drop for Cli {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn json_body(response: &ClientResponse) -> Value {
    serde_json::from_slice(&response.body).expect("response JSON")
}

fn submission() -> Vec<u8> {
    let definition: Value = serde_json::from_slice(include_bytes!(
        "../../../conformance/workflow-v1/valid-strict.json"
    ))
    .expect("existing synthetic definition");
    serde_json::to_vec(&json!({
        "schema_version": "ascension.management/v1",
        "request_id": "cli-blocked-admission",
        "definition": definition,
        "artifact_id": null,
        "instance_id": "sts2-synthetic-1",
        "profile": "synthetic",
        "admission": null
    }))
    .expect("submission")
}

#[test]
fn shipped_cli_reaps_a_sqlite_blocked_backend_and_preserves_submission_recovery() {
    let mut cli = Cli::start();
    let helper = Process::child_of(cli.child.id());
    let backend = Process::child_of(helper.pid);
    let blocker = Connection::open(cli.root.join("workflow.sqlite")).expect("test lock owner");
    blocker
        .execute_batch("BEGIN IMMEDIATE")
        .expect("retain a competing SQLite writer");
    let body = submission();
    let started = Instant::now();
    let refused = cli.request("POST", "/v1/workflow-runs", Some(&body));
    assert!(started.elapsed() < Duration::from_secs(4));
    assert_eq!(refused.status, 503);
    assert_eq!(
        json_body(&refused)["schema_version"],
        "ascension.management-error/v2"
    );
    assert_eq!(json_body(&refused)["error"]["class"], "write_state_unknown");
    assert_eq!(
        json_body(&refused)["error"]["code"],
        "dispatch_backend_terminated"
    );
    assert!(backend.exited(), "the actual SQLite waiter has exited");
    assert!(!helper.exited(), "the same supervisor owns replacement");
    blocker
        .execute_batch("ROLLBACK")
        .expect("release test writer");

    let recovered = cli.request(
        "GET",
        "/v1/workflow-submissions/cli-blocked-admission",
        None,
    );
    assert_eq!(recovered.status, 200);
    assert_eq!(json_body(&recovered)["state"], "missing");
    assert!(json_body(&recovered)["workflow_run_id"].is_null());
    let replacement = Process::child_of(helper.pid);
    assert!(!replacement.exited());
    let submitted = cli.request("POST", "/v1/workflow-runs", Some(&body));
    assert_eq!(submitted.status, 200, "{}", json_body(&submitted));
    let replay = cli.request("POST", "/v1/workflow-runs", Some(&body));
    assert_eq!(replay.status, 200);
    assert_eq!(json_body(&replay), json_body(&submitted));
    let runs: i64 = blocker
        .query_row("SELECT COUNT(*) FROM management_runs", [], |row| row.get(0))
        .expect("retained durable run count");
    assert_eq!(runs, 1);
    let diagnostic_loss = cli.diagnostic_loss();
    cli.stop_frontend();
    helper.await_exit();
    replacement.await_exit();
    let logs = fs::read_to_string(cli.root.join("stderr")).expect("diagnostics");
    assert!(logs.contains("budget_ms=250"));
    assert!(logs.contains("dispatch_timeout") || diagnostic_loss > 0);
    assert!(!logs.contains(TOKEN));
    assert!(!logs.contains("cli-blocked-admission"));
}
