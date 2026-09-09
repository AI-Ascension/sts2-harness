// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};
use sts2_harness::verify_runtime_v4_expert_rest_action_artifact;

pub(crate) const INSTANCE_ID: &str = "instance-1";
pub(crate) const CALLER_ID: &str = "harness";
pub(crate) const SESSION_ID: &str = "gateway-session-1";
pub(crate) const MCP_SESSION_ID: &str = "mcp-session-1";
pub(crate) const LEASE_ID: &str = "lease-1";
pub(crate) const LEASE_EPOCH: u64 = 1;
pub(crate) const REST_SCHEMA_DIGEST: &str =
    "bb3555fae28eb1f79d08a15e9884696a579e4c20836f5016509f17e0f4c36fbd";

#[derive(Clone, Debug)]
pub(crate) struct DownstreamRequest {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Value,
}

#[derive(Clone, Debug)]
pub(crate) struct DownstreamResponse {
    pub(crate) status: u16,
    pub(crate) body: Value,
}

pub(crate) struct DownstreamLedger {
    pub(crate) requests: Vec<DownstreamRequest>,
    pub(crate) responses: Vec<DownstreamResponse>,
    pub(crate) errors: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    SmithOption,
    SmithSelection(u8),
    MendOption,
    MendSelection,
    Victory,
}

impl Phase {
    fn identity(self) -> (&'static str, u64) {
        match self {
            Self::SmithOption => ("live:9", 9),
            Self::SmithSelection(selected) => ("live:10", 10 + u64::from(selected)),
            Self::MendOption => ("live:13", 13),
            Self::MendSelection => ("live:14", 14),
            Self::Victory => ("live:15", 15),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Settlement {
    SmithRequested,
    SmithCardOne,
    SmithCardTwo,
    SmithCompleted,
    MendRequested,
    MendCompleted,
}

#[derive(Clone, Debug)]
struct Operation {
    generation: u64,
    state_id: String,
    action: Value,
    action_id: String,
    unknown: bool,
    settlement: Option<Settlement>,
}

#[derive(Debug)]
struct FixtureState {
    phase: Phase,
    operations: BTreeMap<String, Operation>,
    duplicate_posts: usize,
}

pub(crate) struct ModServer {
    pub(crate) address: SocketAddr,
    stop: Arc<AtomicBool>,
    state: Arc<Mutex<FixtureState>>,
    ledger: Arc<Mutex<DownstreamLedger>>,
    worker: Option<thread::JoinHandle<()>>,
}

/// Check the exact candidate artifact consumed by this executable fixture. The shared verifier
/// checks the complete byte inventory before the fixture starts, rejecting a mixed tree.
pub(crate) fn verify_canonical_inputs() -> Result<(), Box<dyn std::error::Error>> {
    verify_runtime_v4_expert_rest_action_artifact()
        .map_err(|error| format!("canonical REST artifact verification failed: {error}"))?;
    let manifest: Value = serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../protocol-artifact/runtime-v4-expert-rest-action/manifest.json"
    )))?;
    if manifest["schema_digest"] != REST_SCHEMA_DIGEST
        || manifest["protocol_version"] != "runtime-v4-expert-rest-action-v1"
        || manifest["profile"] != "expert-rest-action"
    {
        return Err("canonical REST manifest metadata mismatch".into());
    }
    Ok(())
}

impl ModServer {
    pub(crate) fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(FixtureState {
            phase: Phase::SmithOption,
            operations: BTreeMap::new(),
            duplicate_posts: 0,
        }));
        let ledger = Arc::new(Mutex::new(DownstreamLedger {
            requests: Vec::new(),
            responses: Vec::new(),
            errors: Vec::new(),
        }));
        let worker_stop = Arc::clone(&stop);
        let worker_state = Arc::clone(&state);
        let worker_ledger = Arc::clone(&ledger);
        let worker = thread::spawn(move || {
            while !worker_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let outcome = read_request(&mut stream).and_then(|request| {
                            let response = worker_state
                                .lock()
                                .map_err(|_| "fixture state lock failed".to_owned())
                                .and_then(|mut state| fixture_response(&mut state, &request));
                            match worker_ledger.lock() {
                                Ok(mut ledger) => ledger.requests.push(request.clone()),
                                Err(_) => return Err("fixture ledger lock failed".into()),
                            }
                            match response {
                                Ok((status, body)) => {
                                    if let Ok(mut ledger) = worker_ledger.lock() {
                                        ledger.responses.push(DownstreamResponse {
                                            status,
                                            body: body.clone(),
                                        });
                                    }
                                    Ok(write_response(&mut stream, status, &body)
                                        .map_err(|error| error.to_string())?)
                                }
                                Err(error) => {
                                    if let Ok(mut ledger) = worker_ledger.lock() {
                                        ledger.errors.push(error.clone());
                                        ledger.responses.push(DownstreamResponse {
                                            status: 500,
                                            body: json!({"error_code":"fixture_invalid_request"}),
                                        });
                                    }
                                    Ok(write_response(
                                        &mut stream,
                                        500,
                                        &json!({"error_code":"fixture_invalid_request"}),
                                    )
                                    .map_err(|write_error| write_error.to_string())?)
                                }
                            }
                        });
                        if let Err(error) = outcome
                            && let Ok(mut ledger) = worker_ledger.lock()
                        {
                            ledger.errors.push(error.to_string());
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => {
                        if let Ok(mut ledger) = worker_ledger.lock() {
                            ledger.errors.push(error.to_string());
                        }
                        break;
                    }
                }
            }
        });
        Ok(Self {
            address,
            stop,
            state,
            ledger,
            worker: Some(worker),
        })
    }

    pub(crate) fn finish(mut self) -> DownstreamLedger {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        let mut ledger = self
            .ledger
            .lock()
            .map(|ledger| DownstreamLedger {
                requests: ledger.requests.clone(),
                responses: ledger.responses.clone(),
                errors: ledger.errors.clone(),
            })
            .unwrap_or_else(|_| DownstreamLedger {
                requests: Vec::new(),
                responses: Vec::new(),
                errors: vec![String::from("fixture ledger lock failed")],
            });
        if let Ok(state) = self.state.lock()
            && state.duplicate_posts != 0
        {
            ledger.errors.push(format!(
                "fixture observed {} duplicate operation POST(s)",
                state.duplicate_posts
            ));
        }
        ledger
    }
}

impl Drop for ModServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

include!("runtime_v4_rest_executable_composition_fixture_wire.rs");
