// SPDX-License-Identifier: MIT

//! Drives the pinned `watchdog-host-lease-control-v1` terminal as a served
//! downstream.
//!
//! The vector target proves the canonical form and the proof recipe; this target
//! proves what answers the gateway's recovery hop. It posts signed frames at the
//! in-process terminal of the shared fixture, compares the acknowledgments with
//! the pinned response fixtures, and (for operators) runs the shipped long-lived
//! downstream process the soak campaign launches.

#[path = "support/runtime_v4_executable_composition_fixture.rs"]
#[allow(
    dead_code,
    reason = "the served downstream exercises a subset of the fixture"
)]
mod fixture;
#[path = "support/host_lease_control_support.rs"]
#[allow(dead_code, reason = "this target uses a subset of the shared readers")]
mod support;

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;

use serde_json::{Value, json};

use fixture::host_lease_mux::host_lease_control::{HostLeaseControl, HostLeaseKind};
use fixture::host_lease_mux::host_lease_control_canonical::{
    decode_host_lease_key, parse_canonical_input, proof_for, verify_proof,
};
use support::{MAX_FRAME_BYTES, artifact_bytes, post, vectors, without_exchange_identity};

fn the_served_downstream_answers_a_signed_install() -> Result<(), Box<dyn std::error::Error>> {
    let vectors = vectors()?;
    let key = decode_host_lease_key(
        vectors["test_key_hex"]
            .as_str()
            .ok_or("the pinned vectors carry no test key")?,
    )?;
    let principal = "00000000-0000-4000-8000-00000000000e";
    let server = Arc::new(HostLeaseControl::new(key, principal));
    let kind = HostLeaseKind::from_request_name("lease_install_request")
        .ok_or("the install request kind is not in the fixed table")?;

    let fixture = parse_canonical_input(
        &artifact_bytes("fixtures/valid/lease-install-request.json")?,
        MAX_FRAME_BYTES,
    )?;
    let mut signed = fixture.clone();
    signed["auth"]["proof"] = json!(proof_for(&fixture, kind.request_domain(), &key)?);

    let downstream = fixture::ModServer::bind_with_host_lease(
        "127.0.0.1:0",
        fixture::FixtureMode::Success,
        server,
    )?;
    let address = downstream.address;

    // The terminal refuses the unsigned fixture, because a placeholder proof is
    // not a proof and the profile verifies before any acknowledgment.
    let refused = post(
        address,
        "/api/v1/runtime/recovery",
        &serde_json::to_vec(&fixture)?,
    )?;
    if refused.status == 200 {
        return Err(String::from("the recovery mux answered an unsigned frame").into());
    }

    let response = post(
        address,
        "/api/v1/runtime/recovery",
        &serde_json::to_vec(&signed)?,
    )?;
    if response.status != 200 {
        return Err(format!("the recovery mux refused a signed frame: {response:?}").into());
    }
    if response.body["kind"] != json!("lease_install_response") {
        return Err(format!("the recovery mux answered {}", response.body["kind"]).into());
    }
    let ack = response.body["payload"]["ack"].clone();
    if ack["installation_id"] != fixture["payload"]["installation_id"] {
        return Err(String::from("the acknowledgment did not copy the installation").into());
    }
    if ack["grant_digest"] != fixture["payload"]["grant_digest"] {
        return Err(String::from("the acknowledgment did not copy the grant digest").into());
    }
    if ack["expires_at"] != fixture["payload"]["grant"]["lease"]["expires_at"] {
        return Err(String::from("an install acknowledgment must bind the grant expiry").into());
    }
    if ack["renew_sequence"] != Value::Null {
        return Err(String::from("an install acknowledgment reports no renewal").into());
    }
    if ack["recorded_at"] != fixture["sent_at"] {
        return Err(String::from("the acknowledgment did not copy the frame time").into());
    }
    let proof = response.body["auth"]["proof"]
        .as_str()
        .ok_or("the acknowledgment carries no proof")?;
    verify_proof(&response.body, kind.ack_domain(), &key, proof)?;
    if verify_proof(&response.body, kind.request_domain(), &key, proof).is_ok() {
        return Err(
            String::from("an acknowledgment proof verified under the request domain").into(),
        );
    }
    // Changing any signed field has to invalidate the retained proof.
    let mut tampered = response.body.clone();
    tampered["payload"]["ack"]["recorded_at"] = json!("2000-01-01T00:00:00Z");
    if verify_proof(&tampered, kind.ack_domain(), &key, proof).is_ok() {
        return Err(String::from("a tampered acknowledgment still verified").into());
    }
    let ledger = downstream.finish();
    if !ledger.errors.iter().any(|error| error.contains("proof")) {
        return Err(String::from("the refused frame was not recorded as a proof refusal").into());
    }
    Ok(())
}

/// The three pinned acknowledgments are the host's answers to the three pinned
/// requests, and the profile says every derived field is copied rather than
/// generated. Clearing the exchange identity and comparing the frames turns that
/// rule into a check: a terminal that invents a fence generation, reports a
/// renewal for an install, or binds the wrong expiry fails here.
fn the_acknowledgments_match_the_pinned_responses() -> Result<(), Box<dyn std::error::Error>> {
    let vectors = vectors()?;
    let key = decode_host_lease_key(
        vectors["test_key_hex"]
            .as_str()
            .ok_or("the pinned vectors carry no test key")?,
    )?;
    let terminal = Arc::new(HostLeaseControl::new(
        key,
        "00000000-0000-4000-8000-00000000000e",
    ));
    let downstream = fixture::ModServer::bind_with_host_lease(
        "127.0.0.1:0",
        fixture::FixtureMode::Success,
        terminal,
    )?;
    let cases = [
        (
            "lease_install_request",
            "fixtures/valid/lease-install-request.json",
            "fixtures/valid/lease-install-response.json",
        ),
        (
            "lease_renew_request",
            "fixtures/valid/lease-renew-request.json",
            "fixtures/valid/lease-renew-response.json",
        ),
        (
            "lease_revoke_request",
            "fixtures/valid/lease-revoke-request.json",
            "fixtures/valid/lease-revoke-response.json",
        ),
    ];
    for (name, request, response) in cases {
        let kind = HostLeaseKind::from_request_name(name)
            .ok_or_else(|| format!("{name} is not in the fixed request table"))?;
        let fixture = parse_canonical_input(&artifact_bytes(request)?, MAX_FRAME_BYTES)?;
        let mut signed = fixture.clone();
        signed["auth"]["proof"] = json!(proof_for(&fixture, kind.request_domain(), &key)?);
        let answer = post(
            downstream.address,
            "/api/v1/runtime/recovery",
            &serde_json::to_vec(&signed)?,
        )?;
        if answer.status != 200 {
            return Err(
                format!("{name}: the terminal refused the pinned request: {answer:?}").into(),
            );
        }
        let pinned = parse_canonical_input(&artifact_bytes(response)?, MAX_FRAME_BYTES)?;
        let produced = without_exchange_identity(&answer.body);
        let expected = without_exchange_identity(&pinned);
        if produced != expected {
            return Err(format!(
                "{name}: the acknowledgment derived {produced} where the pin fixes {expected}"
            )
            .into());
        }
    }
    let _ = downstream.finish();
    Ok(())
}

#[test]
fn the_served_host_lease_control_terminal_is_the_pinned_one()
-> Result<(), Box<dyn std::error::Error>> {
    the_served_downstream_answers_a_signed_install()?;
    the_acknowledgments_match_the_pinned_responses()?;
    Ok(())
}

/// Resolve the operator binary the soak campaign launches. The campaign passes
/// this path in `bin_dir`, so the probe has to be told where it is; there is no
/// stable location for a test target's executable.
fn campaign_downstream_binary() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = std::env::var_os("STS2_SYNTHETIC_MOD_SERVER_BINARY")
        .map(PathBuf::from)
        .ok_or("STS2_SYNTHETIC_MOD_SERVER_BINARY is required for this ignored operator test")?;
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!(
            "STS2_SYNTHETIC_MOD_SERVER_BINARY is not a file: {}",
            path.display()
        )
        .into())
    }
}

/// Read the readiness line out of the operator downstream's stdout. The
/// campaign greps the same prefix, so the probe and the campaign agree on what
/// "started" means.
fn campaign_downstream_readiness(
    stdout: std::process::ChildStdout,
) -> Result<String, Box<dyn std::error::Error>> {
    use std::io::BufRead;

    let mut reader = std::io::BufReader::new(stdout);
    for _ in 0..64 {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Err(String::from("the campaign downstream exited before it listened").into());
        }
        if line.contains("synthetic_mod_listening=") {
            return Ok(line);
        }
    }
    Err(String::from("the campaign downstream never reported a listening address").into())
}

fn campaign_address(readiness: &str) -> Result<std::net::SocketAddr, Box<dyn std::error::Error>> {
    readiness
        .split_whitespace()
        .find_map(|field| field.strip_prefix("synthetic_mod_listening="))
        .ok_or_else(|| format!("the readiness line carries no address: {readiness}").into())
        .and_then(|address| Ok(address.parse()?))
}

/// The campaign path end to end: the operator's long-lived downstream process,
/// configured only through the environment, has to terminate the same frames the
/// in-process terminal does. This is the check that fails if the environment
/// half of the sideband is removed, so it runs the shipped operator binary.
///
/// The identity-copy depth, the proof-domain separation, and the tamper
/// assertion stay in `the_served_downstream_answers_a_signed_install`; this probe
/// establishes that the environment-configured process is the same terminal.
#[test]
#[ignore = "operator-only: requires the built synthetic_mod_server operator binary"]
fn the_env_configured_campaign_downstream_answers_a_signed_install()
-> Result<(), Box<dyn std::error::Error>> {
    let binary = campaign_downstream_binary()?;
    let vectors = vectors()?;
    let key_hex = vectors["test_key_hex"]
        .as_str()
        .ok_or("the pinned vectors carry no test key")?;
    let key = decode_host_lease_key(key_hex)?;

    let mut child = Command::new(binary)
        .args([
            "--ignored",
            "--exact",
            "run_synthetic_downstream_until_terminated",
            "--nocapture",
        ])
        .env("STS2_SYNTHETIC_MOD_ADDR", "127.0.0.1:0")
        .env("STS2_SYNTHETIC_HOST_LEASE_KEY", key_hex)
        .stdout(Stdio::piped())
        .spawn()?;
    let outcome = (|| -> Result<(), Box<dyn std::error::Error>> {
        let stdout = child
            .stdout
            .take()
            .ok_or("the campaign downstream has no stdout")?;
        let readiness = campaign_downstream_readiness(stdout)?;
        if !readiness.contains("host_lease=enabled") {
            return Err(format!(
                "the campaign downstream opened its recovery mux without the host lease key: {readiness}"
            )
            .into());
        }
        assert_campaign_install(campaign_address(&readiness)?, &key)
    })();
    let _ = child.kill();
    let _ = child.wait();
    outcome
}

fn assert_campaign_install(
    address: std::net::SocketAddr,
    key: &[u8; 32],
) -> Result<(), Box<dyn std::error::Error>> {
    let kind = HostLeaseKind::from_request_name("lease_install_request")
        .ok_or("the install request kind is not in the fixed table")?;
    let fixture = parse_canonical_input(
        &artifact_bytes("fixtures/valid/lease-install-request.json")?,
        MAX_FRAME_BYTES,
    )?;
    let mut signed = fixture.clone();
    signed["auth"]["proof"] = json!(proof_for(&fixture, kind.request_domain(), key)?);
    let refused = post(
        address,
        "/api/v1/runtime/recovery",
        &serde_json::to_vec(&fixture)?,
    )?;
    if refused.status == 200 {
        return Err(String::from("the operator downstream answered an unsigned frame").into());
    }
    let response = post(
        address,
        "/api/v1/runtime/recovery",
        &serde_json::to_vec(&signed)?,
    )?;
    if response.status != 200 || response.body["kind"] != json!("lease_install_response") {
        return Err(format!("the operator downstream refused a signed frame: {response:?}").into());
    }
    if response.body["payload"]["ack"]["installation_id"] != fixture["payload"]["installation_id"] {
        return Err(String::from("the operator downstream did not copy the installation").into());
    }
    let proof = response.body["auth"]["proof"]
        .as_str()
        .ok_or("the acknowledgment carries no proof")?;
    verify_proof(&response.body, kind.ack_domain(), key, proof)?;
    Ok(())
}
