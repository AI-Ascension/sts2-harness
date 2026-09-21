// SPDX-License-Identifier: MIT

use super::super::super::super::super::runtime_v3_settings::live_admission::admitted_live_episode;
use super::*;
use serde_json::Value;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const INSTANCE: &str = "instance-1";
const CALLER: &str = "harness";
const SESSION: &str = "session-1";
const MCP_SESSION: &str = "mcp-session-1";
const LEASE: &str = "lease-1";
const LEASE_EPOCH: u64 = 1;
const MANIFEST: &str = "content-1";
const LOCALE: &str = "en-US";
/// Generation the synthetic producer issues on every lookup-binding observation.
const BINDING_STATE_GENERATION: u64 = 0;

/// Closed set of producer-side negatives the synthetic mod server can emit while
/// the pinned real Gateway and MCP stay in the path. Each variant changes only the
/// downstream producer bytes; the peers and the candidate runtime are unmodified.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PeerNegative {
    /// Every route answers with the adopted manifest and a consistent generation.
    None,
    /// Lookup-binding and bootstrap carry a manifest the owner never adopted.
    ForeignManifest,
    /// The bootstrap snapshot reports a generation the binding observation never issued.
    StaleGeneration,
    /// The bootstrap route answers with the protocol `not_observable` error shape.
    NotObservable,
}

impl PeerNegative {
    pub(super) const fn content_manifest_id(self) -> &'static str {
        match self {
            Self::ForeignManifest => "foreign-content",
            Self::None | Self::StaleGeneration | Self::NotObservable => MANIFEST,
        }
    }

    pub(super) const fn bootstrap_state_generation(self) -> u64 {
        match self {
            Self::StaleGeneration => BINDING_STATE_GENERATION + 1,
            Self::None | Self::ForeignManifest | Self::NotObservable => BINDING_STATE_GENERATION,
        }
    }
}

pub(super) struct LoggedRequest {
    pub(super) method: String,
    pub(super) path: String,
    pub(super) correlation: Option<String>,
    pub(super) body: Value,
}

pub(super) struct LivePeers {
    address: SocketAddr,
    gateway: Option<Child>,
    stop: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<LoggedRequest>>>,
    worker: Option<JoinHandle<Result<(), String>>>,
}

impl LivePeers {
    pub(super) fn start(gateway_binary: &Path, negative: PeerNegative) -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|_| "synthetic mod listener")?;
        listener
            .set_nonblocking(true)
            .map_err(|_| "synthetic mod listener setup")?;
        let mod_address = listener.local_addr().map_err(|_| "synthetic mod address")?;
        let address = free_address()?;
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let worker = spawn_mod_server(listener, Arc::clone(&stop), Arc::clone(&requests), negative);
        let mut gateway = Command::new(gateway_binary);
        gateway
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("STS2_GATEWAY_ADDR", address.to_string())
            .env("STS2_MOD_ADDR", mod_address.to_string())
            .env("STS2_GATEWAY_TOKEN", "gateway-token")
            .env("STS2_MOD_TOKEN", "mod-token")
            .env("STS2_INSTANCE_ID", INSTANCE)
            .env("STS2_CALLER_ID", CALLER)
            .env("STS2_SESSION_ID", SESSION)
            .env("STS2_MCP_SESSION_ID", MCP_SESSION)
            .env("STS2_LEASE_ID", LEASE)
            .env("STS2_LEASE_EPOCH", LEASE_EPOCH.to_string())
            .env("STS2_GAME_INFORMATION_CONTENT_MANIFEST_ID", MANIFEST)
            .env("STS2_GAME_INFORMATION_RUN_ID", RUN)
            .env("STS2_GAME_INFORMATION_LOCALE", LOCALE)
            // The pinned Gateway installs its live-observation bootstrap handler only
            // on this explicit setting; without it the MCP never offers the bootstrap
            // tool and the candidate runtime fails before any producer bootstrap.
            .env("STS2_GAME_INFORMATION_LIVE_BOOTSTRAP_ENABLED", "true")
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());
        let mut gateway = gateway
            .spawn()
            .map_err(|_| String::from("pinned Gateway process did not start"))?;
        if let Err(error) = wait_until_listening(address, &mut gateway) {
            let _ = gateway.kill();
            let _ = gateway.wait();
            stop.store(true, Ordering::Release);
            let _ = worker.join();
            return Err(error);
        }
        Ok(Self {
            address,
            gateway: Some(gateway),
            stop,
            requests,
            worker: Some(worker),
        })
    }

    pub(super) const fn address(&self) -> SocketAddr {
        self.address
    }

    pub(super) fn assert_no_downstream_request(&self) {
        assert!(
            self.requests
                .lock()
                .expect("synthetic mod request log")
                .is_empty(),
            "runtime must not contact the Gateway producer before operator adoption"
        );
    }

    pub(super) fn finish(mut self) -> Result<Vec<LoggedRequest>, String> {
        if let Some(mut gateway) = self.gateway.take() {
            if gateway.try_wait().map_err(|_| "poll Gateway")?.is_none() {
                gateway.kill().map_err(|_| "stop Gateway")?;
            }
            let _ = gateway.wait().map_err(|_| "reap Gateway")?;
        }
        self.stop.store(true, Ordering::Release);
        if self
            .worker
            .take()
            .ok_or("synthetic mod worker disappeared")?
            .join()
            .map_err(|_| "synthetic mod worker panicked")?
            .is_err()
        {
            return Err(String::from("synthetic mod server failed"));
        }
        let requests = self
            .requests
            .lock()
            .map_err(|_| "synthetic mod request log unavailable")?;
        if admitted_live_episode() {
            eprintln!(
                "real-peer downstream trace: {:?}",
                requests
                    .iter()
                    .map(|request| (&request.method, &request.path, &request.body))
                    .collect::<Vec<_>>()
            );
        }
        Ok(requests
            .iter()
            .map(|request| LoggedRequest {
                method: request.method.clone(),
                path: request.path.clone(),
                correlation: request.correlation.clone(),
                body: request.body.clone(),
            })
            .collect())
    }
}

pub(super) fn pinned_binary(environment: &str) -> Result<std::path::PathBuf, String> {
    let path = std::env::var_os(environment)
        .map(std::path::PathBuf::from)
        .ok_or_else(|| format!("{environment} is required for the real-peer acceptance test"))?;
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!("{environment} is not a file"))
    }
}

fn free_address() -> Result<SocketAddr, String> {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map_err(|_| String::from("loopback test address unavailable"))
}

fn wait_until_listening(address: SocketAddr, child: &mut Child) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if child
            .try_wait()
            .map_err(|_| "poll Gateway startup")?
            .is_some()
        {
            return Err(String::from("pinned Gateway exited during startup"));
        }
        if TcpStream::connect(address).is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(String::from("pinned Gateway startup deadline exceeded"));
        }
        thread::sleep(Duration::from_millis(20));
    }
}

#[path = "runtime_v3_game_information_entry_live_peer_mod_server.rs"]
mod mod_server;

use mod_server::spawn_mod_server;
