// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::net::{SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

#[path = "runtime_v4_executable_composition_fixture/actions.rs"]
mod actions;
#[path = "runtime_v4_executable_composition_fixture/gate.rs"]
mod gate;
#[path = "runtime_v4_executable_composition_fixture/wire.rs"]
mod wire;
pub(crate) use gate::ActionReadGate;
use wire::{read_request, write_response};

#[path = "runtime_v4_executable_composition_fixture/identity.rs"]
pub(crate) mod identity;
pub(crate) use identity::{Admitted, Identity};

#[path = "runtime_v4_executable_composition_fixture/host_lease_mux.rs"]
pub(crate) mod host_lease_mux;

use host_lease_mux::host_lease_control::HostLeaseControl;

pub(crate) const INSTANCE_ID: &str = "instance-1";
pub(crate) const CALLER_ID: &str = "harness";
pub(crate) const SESSION_ID: &str = "gateway-session-1";
pub(crate) const MCP_SESSION_ID: &str = "mcp-session-1";
pub(crate) const LEASE_ID: &str = "lease-1";
pub(crate) const LEASE_EPOCH: u64 = 1;
pub(crate) const ACTION_ID: &str = "potion:7:potion:fire:enemy:1";
pub(crate) const REVIEWED_EXO_REVISION: &str = "b06869ab789dee3f80ca474b5fa89dbe47ccb859";

#[derive(Clone, Copy)]
pub(crate) enum FixtureMode {
    Success,
    UnknownOperation,
    /// The mutation boundary admits the action, the first settlement read is
    /// unresolved, and reconciliation of that same operation settles it.
    /// This is a synthetic downstream sequence for the served cancellation
    /// process test; gateway and MCP remain real peer processes.
    AcceptedBarrierThenSettled,
    ForeignExpertState,
    /// Deliberately violates the closed expert-state envelope after the real
    /// gateway has selected its fixed route.  This is a synthetic downstream
    /// fault, not a replacement for either the gateway or MCP peer.
    MalformedExpertState,
}

#[derive(Clone, Debug)]
pub(crate) struct DownstreamRequest {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Value,
    /// The body bytes exactly as they arrived. The host lease-control profile
    /// validates its canonical input before parsing, so the recovery mux cannot
    /// read the already-normalized projection.
    pub(crate) raw: Vec<u8>,
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

pub(crate) struct ModServer {
    pub(crate) address: SocketAddr,
    stop: Arc<AtomicBool>,
    ledger: Arc<Mutex<DownstreamLedger>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl ModServer {
    pub(crate) fn new(mode: FixtureMode) -> Result<Self, Box<dyn std::error::Error>> {
        Self::bind("127.0.0.1:0", mode)
    }

    /// Bind the synthetic downstream to an explicit address for operator
    /// soak campaigns that run the fixture as a long-lived process.
    #[allow(dead_code, reason = "used by the synthetic_mod_server operator target")]
    pub(crate) fn bind(
        address: &str,
        mode: FixtureMode,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Self::bind_inner(address, mode, None, None)
    }

    pub(crate) fn accepted_barrier_then_settled()
    -> Result<(Self, Arc<ActionReadGate>), Box<dyn std::error::Error>> {
        let gate = Arc::new(ActionReadGate::new());
        let server = Self::bind_inner(
            "127.0.0.1:0",
            FixtureMode::AcceptedBarrierThenSettled,
            Some(Arc::clone(&gate)),
            None,
        )?;
        Ok((server, gate))
    }

    fn bind_inner(
        address: &str,
        mode: FixtureMode,
        gate: Option<Arc<ActionReadGate>>,
        host_lease: Option<Arc<HostLeaseControl>>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let listener = TcpListener::bind(address)?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let action_reads = Arc::new(AtomicU64::new(0));
        let worker_action_reads = Arc::clone(&action_reads);
        let worker_gate = gate.clone();
        let worker_host_lease = host_lease;
        let identity = Arc::new(Identity::pinned());
        let worker_identity = Arc::clone(&identity);
        let ledger = Arc::new(Mutex::new(DownstreamLedger {
            requests: Vec::new(),
            responses: Vec::new(),
            errors: Vec::new(),
        }));
        let worker_ledger = Arc::clone(&ledger);
        let worker = thread::spawn(move || {
            while !worker_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => match read_request(&mut stream) {
                        Ok(request) => {
                            let response = fixture_response(
                                &request,
                                mode,
                                &worker_action_reads,
                                worker_gate.as_deref(),
                                worker_host_lease.as_deref(),
                                &worker_identity,
                            );
                            if let Ok(mut ledger) = worker_ledger.lock() {
                                ledger.requests.push(request);
                            }
                            match response {
                                Ok((status, body)) => {
                                    if let Ok(mut ledger) = worker_ledger.lock() {
                                        ledger.responses.push(DownstreamResponse {
                                            status,
                                            body: body.clone(),
                                        });
                                    }
                                    if let Err(error) = write_response(&mut stream, status, &body)
                                        && let Ok(mut ledger) = worker_ledger.lock()
                                    {
                                        ledger.errors.push(error.to_string());
                                    }
                                }
                                Err(error) => {
                                    if let Ok(mut ledger) = worker_ledger.lock() {
                                        ledger.errors.push(error);
                                    }
                                    let _ = write_response(
                                        &mut stream,
                                        500,
                                        &json!({"error_code":"fixture_invalid_request"}),
                                    );
                                }
                            }
                        }
                        Err(error) => {
                            if let Ok(mut ledger) = worker_ledger.lock() {
                                ledger.errors.push(error.to_string());
                            }
                        }
                    },
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
            ledger,
            worker: Some(worker),
        })
    }

    pub(crate) fn finish(mut self) -> DownstreamLedger {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.ledger.lock().map_or_else(
            |_| DownstreamLedger {
                requests: Vec::new(),
                responses: Vec::new(),
                errors: vec![String::from("downstream ledger lock failed")],
            },
            |ledger| DownstreamLedger {
                requests: ledger.requests.clone(),
                responses: ledger.responses.clone(),
                errors: ledger.errors.clone(),
            },
        )
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

fn fixture_response(
    request: &DownstreamRequest,
    mode: FixtureMode,
    action_reads: &AtomicU64,
    gate: Option<&ActionReadGate>,
    host_lease: Option<&HostLeaseControl>,
    identity: &Identity,
) -> Result<(u16, Value), String> {
    if request.headers.get("authorization").map(String::as_str) != Some("Bearer mod-token") {
        return Err(String::from("downstream mod authorization is missing"));
    }
    // The recovery mux authenticates its own frames under the pinned
    // host-lease-control proof, so it is not fenced with the runtime identity.
    if request.path == "/api/v1/runtime/recovery" {
        return host_lease_mux::recovery_response(&request.raw, host_lease, identity);
    }
    let Some(admitted) = identity.admit(&request.headers) else {
        return Ok((409, identity::stale_fence()));
    };
    match request.path.as_str() {
        "/api/v3/runtime/state" => Ok((
            200,
            v3_response("state_response", &request.headers, &admitted),
        )),
        "/api/v3/runtime/legal-actions" => Ok((
            200,
            v3_response("legal_actions_response", &request.headers, &admitted),
        )),
        "/api/v4/runtime/expert-state" => expert_state_response(mode),
        "/api/v4/runtime/expert-action" => match mode {
            FixtureMode::AcceptedBarrierThenSettled => {
                actions::accepted_action(&request.body, &admitted)
            }
            _ => actions::unknown_action(&request.body, &admitted),
        },
        path if path.starts_with("/api/v4/runtime/expert-actions/") => match mode {
            FixtureMode::UnknownOperation => {
                actions::unknown_operation(path, &request.headers, &admitted)
            }
            FixtureMode::AcceptedBarrierThenSettled => {
                if action_reads.fetch_add(1, Ordering::AcqRel) == 0
                    && let Some(gate) = gate
                {
                    gate.block();
                }
                if gate.is_some_and(ActionReadGate::unsettled) {
                    actions::unknown_operation(path, &request.headers, &admitted)
                } else {
                    actions::settled_action(path, &request.headers, &admitted)
                }
            }
            _ => actions::settled_action(path, &request.headers, &admitted),
        },
        _ => Err(format!("unexpected downstream path: {}", request.path)),
    }
}

fn expert_state_response(mode: FixtureMode) -> Result<(u16, Value), String> {
    match mode {
        FixtureMode::Success
        | FixtureMode::UnknownOperation
        | FixtureMode::AcceptedBarrierThenSettled => {
            Ok((200, expert_observation("live:7", 7, false)))
        }
        FixtureMode::ForeignExpertState => Ok((200, expert_observation("foreign-state", 7, false))),
        FixtureMode::MalformedExpertState => Ok((200, json!({"kind":"expert_observation"}))),
    }
}

/// Answer as the deployment this request was admitted on.
///
/// The gateway compares all five identity fields of a response against the
/// request envelope it forwarded, so the response has to answer as the admitted
/// deployment: a downstream that answers as the fixture default is refused as
/// `runtime_v3_response_invalid` by any gateway that admits another identity.
fn v3_response(kind: &str, headers: &BTreeMap<String, String>, admitted: &Admitted) -> Value {
    let mut value: Value = serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../protocol-artifact/runtime-v3-gameplay/golden/state-response.json"
    )))
    .unwrap_or_else(|_| Value::Object(serde_json::Map::new()));
    value["kind"] = json!(kind);
    value["correlation_id"] = json!(
        headers
            .get("x-sts2-correlation-id")
            .cloned()
            .unwrap_or_default()
    );
    for (field, identity) in admitted.fields() {
        value[field] = identity;
    }
    value["generation"] = json!(7);
    value["state_id"] = json!("live:7");
    value["observation"]["state_id"] = json!("live:7");
    value["observation"]["generation"] = json!(7);
    value["observation"]["state"] = json!({"state":"combat","turn_index":8,"enemies":[]});
    value["legal_actions"] = json!([{"action_id":"end:7","action":{"kind":"end_turn"}}]);
    if kind == "legal_actions_response" {
        value["observation"] = Value::Null;
    }
    value
}

fn expert_observation(state_id: &str, generation: u64, terminal: bool) -> Value {
    let mut value: Value = serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../protocol-artifact/runtime-v4-expert/golden/observation.json"
    )))
    .unwrap_or_else(|_| Value::Object(serde_json::Map::new()));
    value["state_id"] = json!(state_id);
    value["generation"] = json!(generation);
    if terminal {
        value["state"] = json!({"state":"victory"});
        value["legal_actions"] = json!([]);
        value["player"]["potions"] = json!([]);
    } else if let Some(actions) = value["legal_actions"].as_array_mut() {
        actions.sort_by_key(|action| {
            if action["action_id"] == "end:7" {
                0_u8
            } else {
                1_u8
            }
        });
    }
    value
}
