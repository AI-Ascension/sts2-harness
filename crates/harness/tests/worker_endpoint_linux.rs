// SPDX-License-Identifier: MIT

#![cfg(target_os = "linux")]

use sts2_harness::worker_bootstrap::{BOOTSTRAP_MAGIC, WorkerBootstrap};
use sts2_harness::worker_endpoint_linux::from_bootstrap;

fn bootstrap(nonce: &str) -> Result<WorkerBootstrap, Box<dyn std::error::Error>> {
    let mut payload: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../protocol-artifact/worker-bootstrap-v1/valid/linux.json"
    ))?;
    payload["launch_nonce"] = serde_json::Value::String(nonce.to_owned());
    let body = serde_json::to_vec(&payload)?;
    let mut frame = BOOTSTRAP_MAGIC.to_vec();
    frame.extend_from_slice(&u32::try_from(body.len())?.to_be_bytes());
    frame.extend_from_slice(&body);
    Ok(WorkerBootstrap::decode(&frame)?)
}

#[test]
fn owner_vector_selects_the_same_nonce_specific_socket() -> Result<(), Box<dyn std::error::Error>> {
    let first = bootstrap("12345678-1234-4234-8234-123456789abc")?;
    assert_eq!(
        from_bootstrap("/run/worker", &first)?.to_str(),
        Some("/run/worker/ascension-worker-12345678-1234-4234-8234-123456789abc.sock")
    );
    let second = bootstrap("22345678-1234-4234-8234-123456789abc")?;
    assert_ne!(
        from_bootstrap("/run/worker", &first)?,
        from_bootstrap("/run/worker", &second)?
    );
    Ok(())
}

#[test]
fn ambiguous_or_oversized_namespace_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let frame = bootstrap("12345678-1234-4234-8234-123456789abc")?;
    for namespace in [
        "",
        "/",
        "relative",
        "//run",
        "/run/",
        "/run//ipc",
        "/run/./ipc",
        "/run/../ipc",
        "/run\\ipc",
        "/run\nipc",
    ] {
        assert!(from_bootstrap(namespace, &frame).is_err());
    }
    assert!(from_bootstrap(&format!("/{}", "x".repeat(100)), &frame).is_err());
    Ok(())
}
