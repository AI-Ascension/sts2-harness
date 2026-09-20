// SPDX-License-Identifier: MIT

//! Executes the pinned `watchdog-host-lease-control-v1` proof vectors against
//! the harness host terminal.
//!
//! The pin at `protocol-artifact/host-lease-control-v1` carries three
//! canonicalization cases, nine proofs over immutable valid frame fixtures,
//! and the raw JSON inputs the profile must refuse. Nothing here is written
//! from the gateway's implementation: the fixtures, the domains, the expected
//! canonical and signed-message digests, and the expected proofs all come out
//! of the pinned artifact, and the fixture digests pin the exact bytes read.
//!
//! The vectors classify themselves as `test-vectors-not-live-evidence`, so the
//! target that drives the same terminal through the served loopback downstream
//! lives in `host_lease_control_served.rs`, rather than claiming the vectors
//! are a live deployment check.

#[path = "support/runtime_v4_executable_composition_fixture.rs"]
#[allow(dead_code, reason = "the vectors exercise a subset of the fixture")]
mod fixture;
#[path = "support/host_lease_control_support.rs"]
#[allow(dead_code, reason = "this target uses a subset of the shared readers")]
mod support;

use fixture::host_lease_mux::host_lease_control_canonical::{
    canonical_hcj1, decode_host_lease_key, parse_canonical_input, proof_for, verify_proof,
};
use support::{MAX_FRAME_BYTES, artifact_bytes, entries, sha256_hex, vectors};

fn the_pinned_canonical_cases_are_reproduced() -> Result<(), Box<dyn std::error::Error>> {
    let vectors = vectors()?;
    let cases = entries(&vectors, "canonical_cases")?;
    if cases.len() != 3 {
        return Err(format!("expected 3 canonical cases, found {}", cases.len()).into());
    }
    for case in cases {
        let name = case["name"]
            .as_str()
            .ok_or("a canonical case has no name")?;
        let canonical = canonical_hcj1(&case["input"])?;
        let expected = case["canonical_sha256"]
            .as_str()
            .ok_or("a canonical case has no digest")?;
        if sha256_hex(&canonical) != expected {
            return Err(format!("{name}: the canonical form does not match the vector").into());
        }
    }
    Ok(())
}

fn the_pinned_frame_proofs_are_reproduced() -> Result<(), Box<dyn std::error::Error>> {
    let vectors = vectors()?;
    let key = decode_host_lease_key(
        vectors["test_key_hex"]
            .as_str()
            .ok_or("the pinned vectors carry no test key")?,
    )?;
    let cases = entries(&vectors, "frame_proof_cases")?;
    if cases.len() != 9 {
        return Err(format!("expected 9 frame proof cases, found {}", cases.len()).into());
    }
    for case in cases {
        let fixture = case["fixture"].as_str().ok_or("a case has no fixture")?;
        let domain = case["domain"].as_str().ok_or("a case has no domain")?;
        let raw = artifact_bytes(fixture)?;
        if sha256_hex(&raw) != case["fixture_sha256"].as_str().unwrap_or_default() {
            return Err(format!("{fixture}: the fixture bytes are not the pinned ones").into());
        }
        let frame = parse_canonical_input(&raw, MAX_FRAME_BYTES)?;
        let mut unsigned = frame.clone();
        unsigned["auth"]
            .as_object_mut()
            .ok_or("a valid fixture carries no authentication")?
            .remove("proof");
        let canonical = canonical_hcj1(&unsigned)?;
        if sha256_hex(&canonical) != case["canonical_sha256"].as_str().unwrap_or_default() {
            return Err(format!("{fixture}: the canonical bytes do not match").into());
        }
        let mut message = domain.as_bytes().to_vec();
        message.push(0);
        message.extend_from_slice(&canonical);
        if sha256_hex(&message) != case["signed_message_sha256"].as_str().unwrap_or_default() {
            return Err(format!("{fixture}: the signed message does not match").into());
        }
        let expected = case["proof"].as_str().ok_or("a case has no proof")?;
        if proof_for(&frame, domain, &key)? != expected {
            return Err(format!("{fixture}: the proof does not match the vector").into());
        }
        // The fixtures ship placeholder proofs, and the profile states they are
        // not valid proofs. Verification has to refuse them, which is what
        // makes every other proof assertion here meaningful.
        let placeholder = frame["auth"]["proof"]
            .as_str()
            .ok_or("a valid fixture carries no proof field")?;
        if placeholder == expected {
            return Err(format!("{fixture}: the fixture already carries the pinned proof").into());
        }
        if verify_proof(&frame, domain, &key, placeholder).is_ok() {
            return Err(format!("{fixture}: a placeholder proof verified").into());
        }
        // A proof is bound to one domain: no other operation's domain accepts it.
        for other in [
            "host-lease-control/v1/lease-install-request",
            "host-lease-control/v1/lease-renew-ack",
            "host-lease-control/v1/lease-revoke-ack",
        ] {
            if other != domain && verify_proof(&frame, other, &key, expected).is_ok() {
                return Err(format!("{fixture}: the proof verified under {other}").into());
            }
        }
    }
    Ok(())
}

fn the_raw_inputs_the_profile_refuses_are_refused() -> Result<(), Box<dyn std::error::Error>> {
    let vectors = vectors()?;
    let refused = vectors["reject_raw_json"]
        .as_array()
        .ok_or("the pinned vectors carry no rejected inputs")?;
    if refused.len() != 8 {
        return Err(format!("expected 8 rejected inputs, found {}", refused.len()).into());
    }
    for raw in refused {
        let raw = raw.as_str().ok_or("a rejected input is not a string")?;
        if parse_canonical_input(raw.as_bytes(), MAX_FRAME_BYTES).is_ok() {
            return Err(format!("the profile admitted {raw}, which it must refuse").into());
        }
    }
    // The control: the same reader accepts a canonical input.
    let accepted = parse_canonical_input(br#"{"a":1,"b":[null,true,false]}"#, MAX_FRAME_BYTES)?;
    if canonical_hcj1(&accepted)? != br#"{"a":1,"b":[null,true,false]}"# {
        return Err(String::from("the canonical reader refused a canonical input").into());
    }
    Ok(())
}

#[test]
fn the_pinned_host_lease_control_vectors_are_reproduced() -> Result<(), Box<dyn std::error::Error>>
{
    the_pinned_canonical_cases_are_reproduced()?;
    the_pinned_frame_proofs_are_reproduced()?;
    the_raw_inputs_the_profile_refuses_are_refused()?;
    Ok(())
}
