// SPDX-License-Identifier: MIT

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, sync_channel};
use std::time::{Duration, Instant};

use serde_json::Value;
use sts2_harness::management::ManagementClient;

const DEADLINE: Duration = Duration::from_secs(15);
const PREFIX: &str = "PROCESS_EVIDENCE ";

pub(crate) fn emit(value: Value) {
    println!("{PREFIX}{value}");
    std::io::stdout().flush().expect("flush evidence");
}

pub(crate) struct Worker {
    child: Child,
    messages: Receiver<Value>,
    reader: Option<std::thread::JoinHandle<()>>,
    pub(crate) client: ManagementClient,
    pub(crate) unauthorized: ManagementClient,
}

impl Worker {
    pub(crate) fn start() -> Self {
        let mut child = Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "recording_server_worker",
                "--ignored",
                "--nocapture",
            ])
            .env("STS2_RECORDING_PROCESS_TEST", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn recording worker");
        let stdout = child.stdout.take().expect("worker stdout");
        let (sender, messages) = sync_channel(8);
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if let Some((_, value)) = line.split_once(PREFIX) {
                    let value = serde_json::from_str(value).expect("worker evidence JSON");
                    if sender.try_send(value).is_err() {
                        break;
                    }
                }
            }
        });
        // Install the cleanup guard before waiting for startup evidence.
        let placeholder = "127.0.0.1:1".parse().expect("placeholder address");
        let mut worker = Self {
            child,
            messages,
            reader: Some(reader),
            client: ManagementClient::new(placeholder, "recording-test-token").expect("client"),
            unauthorized: ManagementClient::new(placeholder, "wrong-token").expect("client"),
        };
        let ready = worker.receive();
        assert_ne!(ready["pid"], std::process::id());
        let address = ready["address"]
            .as_str()
            .expect("address")
            .parse()
            .expect("socket");
        worker.client = ManagementClient::new(address, "recording-test-token").expect("client");
        worker.unauthorized = ManagementClient::new(address, "wrong-token").expect("client");
        worker
    }

    fn receive(&self) -> Value {
        self.messages
            .recv_timeout(DEADLINE)
            .expect("bounded worker response")
    }

    pub(crate) fn evidence(&mut self) -> Value {
        writeln!(self.child.stdin.as_mut().expect("worker stdin"), "snapshot")
            .expect("request evidence");
        self.receive()
    }

    pub(crate) fn finish(&mut self) {
        writeln!(self.child.stdin.as_mut().expect("worker stdin"), "shutdown")
            .expect("shutdown request");
        let deadline = Instant::now() + DEADLINE;
        loop {
            if let Some(status) = self.child.try_wait().expect("worker status") {
                assert!(status.success(), "recording worker failed: {status}");
                return;
            }
            assert!(Instant::now() < deadline, "worker shutdown deadline");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}
