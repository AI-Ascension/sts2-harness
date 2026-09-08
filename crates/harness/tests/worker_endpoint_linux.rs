// SPDX-License-Identifier: MIT

#![cfg(target_os = "linux")]

use sts2_harness::worker_bootstrap::{BOOTSTRAP_MAGIC, WorkerBootstrap};
use sts2_harness::worker_endpoint_linux::from_bootstrap;

#[test]
fn frozen_owner_machine_vectors_match_linux_consumer() -> Result<(), Box<dyn std::error::Error>> {
    use sha2::{Digest, Sha256};
    let bytes = include_bytes!("../../../protocol-artifact/worker-endpoint-v1/conformance.json");
    assert_eq!(
        format!("{:x}", Sha256::digest(bytes)),
        "2f7d9758a681ed77d633e3ed576de1a415734b7ef96255f099e0cb6c9b658f41"
    );
    let document: serde_json::Value = serde_json::from_slice(bytes)?;
    assert_eq!(document["contract"], "ascension-worker-endpoint-v1");
    assert_eq!(document["owner"], "AI-Ascension/ascension-watchdog");
    assert_eq!(document["license"], "MIT");
    let cases = document["cases"]
        .as_array()
        .ok_or("missing conformance cases")?;
    assert_eq!(cases.len(), 27);
    let mut checked = 0;
    for case in cases {
        if case["platform"] == "windows" {
            continue;
        }
        assert_eq!(case["platform"], "linux");
        let nonce = case["nonce"].as_str().ok_or("missing conformance nonce")?;
        let namespace = case["namespace"]
            .as_str()
            .ok_or("missing conformance namespace")?;
        let result = bootstrap(nonce)
            .and_then(|frame| from_bootstrap(namespace, &frame).map_err(Into::into));
        match case["expected"].as_str() {
            Some(expected) => assert_eq!(result?.to_str(), Some(expected), "{}", case["id"]),
            None => assert!(result.is_err(), "{}", case["id"]),
        }
        checked += 1;
    }
    assert_eq!(checked, 19);
    Ok(())
}

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
