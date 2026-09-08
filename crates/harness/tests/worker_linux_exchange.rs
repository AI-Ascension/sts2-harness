// SPDX-License-Identifier: MIT
#![cfg(target_os = "linux")]

#[path = "worker_local_linux_support.rs"]
mod support;

mod execution {
    pub use sts2_harness::WorkerOwnerProof;
}
mod worker_frame_io {
    pub use sts2_harness::worker_frame_io::*;
}
mod worker_handoff {
    pub use sts2_harness::worker_handoff::*;
}
#[path = "../src/worker_linux_exchange.rs"]
mod worker_linux_exchange;
#[path = "../src/worker_local_linux.rs"]
mod worker_local_linux;

use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Read;
use sts2_harness::worker_frame_io::ConnectionDeadline;
use sts2_harness::worker_handoff::{ProbeReply, WorkerCapability, WorkerReply};
use support::{Fixture, IDENTITY_DEADLINE, TestResult, read_frame, write_auth, write_frame};
use tokio::net::UnixStream;
use worker_linux_exchange::LinuxWorkerExchange;
use worker_local_linux::{AUTH_MAGIC, LinuxPeerIdentity, LinuxTransportError, LinuxWorkerConfig};

const PROBE: &[u8] = include_bytes!(
    "../../../protocol-artifact/watchdog-worker-v1/fixtures/valid/probe-request.json"
);

fn config(fixture: &Fixture) -> TestResult<LinuxWorkerConfig> {
    let pid = std::process::id();
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let close = stat.rfind(')').ok_or("missing process comm terminator")?;
    let start = stat
        .get(close + 2..)
        .and_then(|suffix| suffix.split_whitespace().nth(19))
        .ok_or("missing process start token")?
        .parse()?;
    let executable = std::env::current_exe()?;
    let mut image = File::open(&executable)?;
    let mut hash = Sha256::new();
    let mut bytes = [0_u8; 16 * 1024];
    loop {
        let count = image.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        hash.update(&bytes[..count]);
    }
    Ok(LinuxWorkerConfig::new(
        fixture.endpoint.clone(),
        fixture.credential.clone(),
        LinuxPeerIdentity::new(
            rustix::process::getuid().as_raw(),
            pid,
            start,
            executable,
            hash.finalize().into(),
        )?,
    )?)
}

#[tokio::test]
async fn native_exchange_preserves_correlation_and_rejects_wrong_capability() -> TestResult {
    for capability in [WorkerCapability::Probe, WorkerCapability::Dispatch] {
        let fixture = Fixture::new(b"worker-control-secret")?;
        let listener = config(&fixture)?.bind()?;
        let mut client = UnixStream::connect(&fixture.endpoint).await?;
        write_auth(&mut client, &fixture.secret, AUTH_MAGIC).await?;
        write_frame(&mut client, PROBE).await?;
        let exchange = LinuxWorkerExchange::accept(
            &listener,
            ConnectionDeadline::start(IDENTITY_DEADLINE)?,
            capability,
        )
        .await;
        if capability == WorkerCapability::Dispatch {
            assert!(matches!(exchange, Err(LinuxTransportError::Credential)));
            continue;
        }
        let exchange = exchange?;
        let request_id = exchange.request().request().fields()["request_id"].clone();
        exchange
            .write_reply(
                "66666666-6666-4666-8666-666666666666",
                WorkerReply::Probe(ProbeReply {
                    deployment_id: "deployment-1".into(),
                    worker_owner_id: "harness".into(),
                    worker_profile_digest: "a".repeat(64),
                    release_digest: "b".repeat(64),
                    config_digest: "c".repeat(64),
                    ready: false,
                }),
            )
            .await?;
        let response: serde_json::Value = serde_json::from_slice(&read_frame(&mut client).await?)?;
        assert_eq!(response["request_id"], request_id);
        assert_eq!(response["direction"], "response");
        assert_eq!(response["command"], "probe");
    }
    Ok(())
}
