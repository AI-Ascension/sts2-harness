// SPDX-License-Identifier: MIT

//! #651's regression scenario: readiness is decided by the service's identity, not by a connect.
//!
//! Split out of `served/session.rs` for file size. `runtime_v4_executable_composition` re-exports
//! these under their own test names, so they are discovered and run exactly once.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::health_probe::is_workflow_service_health;
use super::wait_for_workflow_service;
// This module sits under `served::session`, so the shared process support — which re-exports
// `reserve` — is two levels up rather than reachable as `crate::process`: these files are
// compiled as modules of the integration-test binary, not as a library with that path.
use super::super::super::reserve;
use sts2_harness::management::{ClientResponse, MANAGEMENT_SCHEMA_VERSION};

/// How long the stand-in child is held open. The real service is never started in these tests, so
/// the child exists only to keep `wait_for_workflow_service`'s "child exited" arm out of the
/// picture and leave the deadline as the only way the probe can end.
const IDLE_CHILD_SECONDS: u64 = 30;

/// #651's regression: a listener that merely answers is not the service we spawned.
///
/// `wait_for_workflow_service` used to accept any completed exchange on the workflow address,
/// because `ManagementClient::request_json` is `Ok` for *any* response the peer manages to
/// produce. That makes the probe satisfiable by a squatter: any other listener on the address —
/// a sibling scenario that drew the same port, a stale process, the synthetic downstream — is
/// indistinguishable from the real service. When that happened the scenario kept talking to the
/// impostor and failed much later, on an unrelated assertion about the provider fixture's error
/// *identity*, naming neither the theft nor the impostor.
///
/// The squatter here is placed on the address before the probe runs, which is the state #651
/// observed: the synthetic downstream had taken `workflow_address` and the real harness child
/// had been refused at its own `bind`. This test makes that state deterministic instead of
/// leaving it to a rare kernel allocation.
pub(crate) fn the_workflow_readiness_probe_rejects_an_impostor_that_answers()
-> Result<(), Box<dyn std::error::Error>> {
    let reserved = reserve()?;
    // The reservation is given up rather than held: the impostor *is* the squatter here, so it
    // has to be the process that owns the port, exactly as the real service would be. Holding the
    // reservation and then binding the impostor would be refused with `EADDRINUSE` and the test
    // would prove nothing.
    let address = reserved.release();
    let (impostor, stop) = answering_impostor(address)?;

    let mut child = std::process::Command::new("/bin/sh")
        .args(["-c", &format!("sleep {IDLE_CHILD_SECONDS}")])
        .spawn()?;

    let outcome = wait_for_workflow_service(&mut child, address);

    // Tear both down before asserting, so a failing assertion cannot leave them running.
    let _ = child.kill();
    let _ = child.wait();
    stop.store(true, Ordering::Release);
    let _ = impostor.join();

    let error = match outcome {
        Ok(_) => {
            return Err(
                "the readiness probe adopted the impostor: it returned a client for a listener \
                 that is not the workflow service"
                    .into(),
            );
        }
        Err(error) => error.to_string(),
    };
    assert!(
        error.contains("answering for something else"),
        "the probe rejected the impostor but did not report it as one: {error}"
    );
    Ok(())
}

/// The other direction: the service's own health answer must be accepted.
///
/// Without this, a predicate that rejected *every* answer would also pass the test above. Paired
/// with the impostor assertions below, the two directions pin the predicate to identity rather
/// than to any particular accept/reject rule.
pub(crate) fn the_workflow_health_predicate_accepts_only_the_services_own_envelope()
-> Result<(), Box<dyn std::error::Error>> {
    let health_envelope = serde_json::json!({
        "schema_version": MANAGEMENT_SCHEMA_VERSION,
        "status": "ok",
    });
    let accepted = ClientResponse {
        status: 200,
        body: serde_json::to_vec(&health_envelope)?,
    };
    assert!(
        is_workflow_service_health(&accepted),
        "the service's own /v1/health answer must satisfy the readiness predicate"
    );

    // The synthetic downstream's answer, at the same status. This is the body #651's squatter
    // returned, and the one the old probe accepted.
    let impostor = ClientResponse {
        status: 200,
        body: serde_json::to_vec(&serde_json::json!({
            "error_code": "fixture_invalid_request",
        }))?,
    };
    assert!(
        !is_workflow_service_health(&impostor),
        "a 200 carrying the fixture's error body must not satisfy the readiness predicate"
    );

    // A non-200 carrying the real envelope is still not the service being ready, so the status
    // half of the predicate is load-bearing rather than incidental.
    let refused = ClientResponse {
        status: 503,
        body: serde_json::to_vec(&health_envelope)?,
    };
    assert!(
        !is_workflow_service_health(&refused),
        "the health envelope at a non-200 status must not satisfy the readiness predicate"
    );

    Ok(())
}

/// Answer every request on `address` with a 200 whose body is the fixture's error shape.
///
/// The accept loop is non-blocking and polls a stop flag, because a blocking `incoming()` cannot
/// be interrupted: dropping another handle to the listener would not end the loop, since the
/// owning handle has already moved onto this thread. The returned flag shuts it down.
fn answering_impostor(
    address: SocketAddr,
) -> Result<(std::thread::JoinHandle<()>, Arc<AtomicBool>), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind(address)?;
    listener.set_nonblocking(true)?;
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = Arc::clone(&stop);
    let thread = std::thread::spawn(move || {
        while !stopping.load(Ordering::Acquire) {
            let stream = match listener.accept() {
                Ok((stream, _)) => stream,
                // Nothing waiting yet; the impostor is idle between probes.
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(_) => break,
            };
            let mut stream = stream;
            if stream.set_nonblocking(false).is_err() {
                break;
            }
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request);
            let payload = serde_json::json!({"error_code": "fixture_invalid_request"}).to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\
                 content-length: {}\r\nconnection: close\r\n\r\n{payload}\r\n",
                payload.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });
    Ok((thread, stop))
}
