// SPDX-License-Identifier: MIT

use std::io::ErrorKind;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

use super::fixture::{CALLER_ID, INSTANCE_ID, LEASE_ID, MCP_SESSION_ID, SESSION_ID};
use super::http_contract::{header, read_request, write_response};

type Result<T = (), E = Box<dyn std::error::Error>> = std::result::Result<T, E>;
type WorkerResult = std::result::Result<GatewayEvidence, &'static str>;

const ALLOCATE_DEADLINE: Duration = Duration::from_secs(15);
const RELEASE_DEADLINE: Duration = Duration::from_secs(10);
const CLEANUP_DEADLINE: Duration = Duration::from_secs(3);
const POLL: Duration = Duration::from_millis(5);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GatewayIdentity {
    pub(crate) instance_id: &'static str,
    pub(crate) caller_id: &'static str,
    pub(crate) session_id: &'static str,
    pub(crate) lease_id: &'static str,
    pub(crate) lease_epoch: u64,
}

impl GatewayIdentity {
    fn expected() -> Self {
        Self {
            instance_id: INSTANCE_ID,
            caller_id: CALLER_ID,
            session_id: SESSION_ID,
            lease_id: LEASE_ID,
            lease_epoch: 1,
        }
    }
}

#[derive(Debug)]
pub(crate) struct GatewayEvidence {
    pub(crate) allocation: Option<GatewayIdentity>,
    pub(crate) accepted_release_count: u8,
    pub(crate) release_status: Option<&'static str>,
}

impl GatewayEvidence {
    pub(crate) fn confirms_exact_release(&self) -> bool {
        self.allocation.as_ref() == Some(&GatewayIdentity::expected())
            && self.accepted_release_count == 1
            && self.release_status == Some("released")
    }
}

pub(crate) struct GatewayPeer {
    pub(crate) address: SocketAddr,
    listener: Option<TcpListener>,
    cancel: Arc<AtomicBool>,
    result: Option<Receiver<WorkerResult>>,
    worker: Option<JoinHandle<()>>,
    started: Option<Instant>,
}

impl GatewayPeer {
    pub(crate) fn bind() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        Ok(Self {
            address,
            listener: Some(listener),
            cancel: Arc::new(AtomicBool::new(false)),
            result: None,
            worker: None,
            started: None,
        })
    }

    pub(crate) fn start(&mut self) -> Result<()> {
        let listener = self
            .listener
            .take()
            .ok_or("Gateway fixture listener was already started")?;
        let cancel = Arc::clone(&self.cancel);
        let (sender, receiver) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name(String::from("h391-synthetic-gateway"))
            .spawn(move || {
                let outcome = serve_gateway(listener, &cancel);
                let _ = sender.send(outcome);
            })?;
        self.started = Some(Instant::now());
        self.result = Some(receiver);
        self.worker = Some(worker);
        Ok(())
    }

    pub(crate) fn finish(&mut self, require_release: bool) -> Result<GatewayEvidence> {
        let receiver = self
            .result
            .take()
            .ok_or("Gateway fixture was not started")?;
        let deadline = if require_release {
            self.started
                .and_then(|started| started.checked_add(ALLOCATE_DEADLINE + RELEASE_DEADLINE))
                .ok_or("Gateway overall deadline is invalid")?
        } else {
            self.cancel.store(true, Ordering::Release);
            Instant::now()
                .checked_add(CLEANUP_DEADLINE)
                .ok_or("Gateway cleanup deadline is invalid")?
        };
        let outcome =
            match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(outcome) => outcome,
                Err(RecvTimeoutError::Timeout) if !require_release => {
                    return Err("Gateway cleanup deadline elapsed".into());
                }
                Err(RecvTimeoutError::Timeout) => {
                    self.cancel.store(true, Ordering::Release);
                    let cleanup = receiver.recv_timeout(CLEANUP_DEADLINE);
                    match cleanup {
                        Ok(_) => {
                            self.join_worker()?;
                            return Err("Gateway completion deadline elapsed".into());
                        }
                        Err(RecvTimeoutError::Disconnected) => {
                            self.join_worker()?;
                            return Err("Gateway worker ended during cleanup".into());
                        }
                        Err(RecvTimeoutError::Timeout) => {
                            return Err("Gateway worker cleanup deadline elapsed".into());
                        }
                    }
                }
                Err(RecvTimeoutError::Disconnected) => {
                    self.join_worker()?;
                    return Err("Gateway worker ended without a result".into());
                }
            };
        self.join_worker()?;
        let evidence = outcome.map_err(|code| code.to_owned())?;
        if require_release && !evidence.confirms_exact_release() {
            return Err("Gateway did not observe exactly one matching release".into());
        }
        Ok(evidence)
    }

    fn join_worker(&mut self) -> Result<()> {
        let worker = self
            .worker
            .take()
            .ok_or("Gateway worker was already joined")?;
        worker
            .join()
            .map_err(|_| "Gateway worker panicked after bounded completion")?;
        Ok(())
    }
}

impl Drop for GatewayPeer {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            self.cancel.store(true, Ordering::Release);
            let _ = worker.join();
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AllocationRequest {
    instance_id: String,
    caller_id: String,
    session_id: String,
}

#[derive(Serialize)]
struct AllocationReply {
    status: &'static str,
    instance_id: &'static str,
    caller_id: &'static str,
    session_id: &'static str,
    lease_id: &'static str,
    lease_epoch: u64,
}

#[derive(Serialize)]
struct ReleaseReply {
    status: &'static str,
}

fn serve_gateway(listener: TcpListener, cancel: &AtomicBool) -> WorkerResult {
    let allocation_deadline = Instant::now()
        .checked_add(ALLOCATE_DEADLINE)
        .ok_or("Gateway allocation deadline invalid")?;
    let Some(mut stream) = accept_until(&listener, allocation_deadline, cancel)? else {
        return Ok(empty_evidence());
    };
    stream
        .set_nonblocking(true)
        .map_err(|_| "Gateway stream setup failed")?;
    let request = read_request(&mut stream, allocation_deadline, cancel)?;
    if request.path != "/v1/sessions/allocate" {
        return Err("Gateway allocation route refused");
    }
    let allocation: AllocationRequest =
        serde_json::from_slice(&request.body).map_err(|_| "Gateway allocation JSON refused")?;
    if allocation.instance_id != INSTANCE_ID
        || allocation.caller_id != CALLER_ID
        || allocation.session_id != SESSION_ID
        || header(&request.headers, "x-mcp-session-id") != Some(MCP_SESSION_ID)
    {
        return Err("Gateway allocation identity refused");
    }
    let identity = GatewayIdentity::expected();
    let response = serde_json::to_vec(&AllocationReply {
        status: "allocated",
        instance_id: identity.instance_id,
        caller_id: identity.caller_id,
        session_id: identity.session_id,
        lease_id: identity.lease_id,
        lease_epoch: identity.lease_epoch,
    })
    .map_err(|_| "Gateway allocation response encoding failed")?;
    write_response(&mut stream, &response, allocation_deadline, cancel)?;
    let mut evidence = GatewayEvidence {
        allocation: Some(identity),
        accepted_release_count: 0,
        release_status: None,
    };
    let release_deadline = Instant::now()
        .checked_add(RELEASE_DEADLINE)
        .ok_or("Gateway release deadline invalid")?;
    let Some(mut stream) = accept_until(&listener, release_deadline, cancel)? else {
        return Ok(evidence);
    };
    stream
        .set_nonblocking(true)
        .map_err(|_| "Gateway stream setup failed")?;
    let request = read_request(&mut stream, release_deadline, cancel)?;
    if request.path != "/v1/instances/entry-instance/release" {
        return Err("Gateway release route refused");
    }
    let release: Value =
        serde_json::from_slice(&request.body).map_err(|_| "Gateway release JSON refused")?;
    if release != Value::Null
        || header(&request.headers, "x-mcp-session-id") != Some(MCP_SESSION_ID)
    {
        return Err("Gateway release identity refused");
    }
    let response = serde_json::to_vec(&ReleaseReply { status: "released" })
        .map_err(|_| "Gateway release response encoding failed")?;
    write_response(&mut stream, &response, release_deadline, cancel)?;
    evidence.accepted_release_count = 1;
    evidence.release_status = Some("released");
    Ok(evidence)
}

fn empty_evidence() -> GatewayEvidence {
    GatewayEvidence {
        allocation: None,
        accepted_release_count: 0,
        release_status: None,
    }
}

fn accept_until(
    listener: &TcpListener,
    deadline: Instant,
    cancel: &AtomicBool,
) -> std::result::Result<Option<TcpStream>, &'static str> {
    loop {
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        match listener.accept() {
            Ok((stream, address)) if address.ip().is_loopback() => return Ok(Some(stream)),
            Ok(_) => return Err("Gateway peer address refused"),
            Err(error) if error.kind() == ErrorKind::WouldBlock => pause(deadline, cancel)?,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(_) => return Err("Gateway accept failed"),
        }
        if Instant::now() >= deadline {
            return Err("Gateway accept deadline elapsed");
        }
    }
}

fn pause(deadline: Instant, cancel: &AtomicBool) -> std::result::Result<(), &'static str> {
    if cancel.load(Ordering::Acquire) {
        return Err("Gateway accept cancelled");
    }
    if Instant::now() >= deadline {
        return Err("Gateway accept deadline elapsed");
    }
    thread::sleep(POLL.min(deadline.saturating_duration_since(Instant::now())));
    Ok(())
}
