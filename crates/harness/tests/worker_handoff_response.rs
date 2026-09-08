// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use std::path::PathBuf;
use sts2_harness::worker_handoff::{
    AcknowledgmentStatus, DispatchReply, LookupReply, ProbeReply, TerminalCompletion,
    TerminalRecord, TerminalStatus, WorkerReply, WorkerRequest,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;
const BOOT: &str = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";

fn fixture(name: &str) -> Result<Vec<u8>, std::io::Error> {
    std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../protocol-artifact/watchdog-worker-v1/fixtures/valid/{name}.json"
    )))
}

fn request(name: &str) -> Result<WorkerRequest, Box<dyn std::error::Error>> {
    Ok(WorkerRequest::decode(&fixture(name)?)?)
}

fn terminal(request: &WorkerRequest) -> Result<TerminalRecord, Box<dyn std::error::Error>> {
    Ok(TerminalRecord::new(
        request,
        TerminalCompletion {
            status: TerminalStatus::Completed,
            checkpoint_sequence: 7,
            terminal_ref: "terminal-1".into(),
            result_digest: "9".repeat(64),
        },
    )?)
}

fn validate_schema(bytes: &[u8]) -> TestResult {
    let schema: Value = serde_json::from_slice(include_bytes!(
        "../../../protocol-artifact/watchdog-worker-v1/schema.json"
    ))?;
    let validator = jsonschema::validator_for(&schema)?;
    let response: Value = serde_json::from_slice(bytes)?;
    assert!(validator.is_valid(&response), "{}", response);
    Ok(())
}

#[test]
fn generated_responses_match_owner_nonterminal_goldens() -> TestResult {
    let cases = [
        (
            "probe-request",
            "probe-response",
            WorkerReply::Probe(ProbeReply {
                deployment_id: "watchdog-deployment-1".into(),
                worker_owner_id: "harness".into(),
                worker_profile_digest: "1".repeat(64),
                release_digest: "2".repeat(64),
                config_digest: "3".repeat(64),
                ready: true,
            }),
        ),
        (
            "lookup-request",
            "lookup-response",
            WorkerReply::Lookup(LookupReply::Running),
        ),
        (
            "acknowledge-request",
            "acknowledge-response",
            WorkerReply::Acknowledge(AcknowledgmentStatus::Acknowledged),
        ),
        (
            "control-request",
            "control-response",
            WorkerReply::Control { accepted: true },
        ),
    ];
    for (input, output, body) in cases {
        let bytes = request(input)?.encode_response(BOOT, body)?;
        validate_schema(&bytes)?;
        assert_eq!(
            serde_json::from_slice::<Value>(&bytes)?,
            serde_json::from_slice::<Value>(&fixture(output)?)?,
            "{input}"
        );
    }
    Ok(())
}

#[test]
fn terminal_dispatch_matches_the_owners_distinct_request_correlation() -> TestResult {
    // The frozen dispatch response is a terminal lookup-on-dispatch example,
    // not the response to the separate 4444 request fixture.
    let expected: Value = serde_json::from_slice(&fixture("dispatch-response")?)?;
    let mut input: Value = serde_json::from_slice(&fixture("dispatch")?)?;
    input["request_id"] = json!("55555555-5555-4555-8555-555555555555");
    let request = WorkerRequest::decode(&serde_json::to_vec(&input)?)?;
    let terminal = TerminalRecord::decode(&serde_json::to_vec(&expected["terminal"])?)?;
    assert_eq!(
        terminal.acknowledgment_digest()?,
        "a0db0fc348623db806d2d053d7724198acb5c5c79f5b3422bf75faf3f653c260"
    );
    let bytes = request.encode_response(
        BOOT,
        WorkerReply::Dispatch(DispatchReply::Terminal(terminal)),
    )?;
    validate_schema(&bytes)?;
    assert_eq!(serde_json::from_slice::<Value>(&bytes)?, expected);
    Ok(())
}

#[test]
fn all_typed_statuses_emit_conformant_shapes() -> TestResult {
    let dispatch = request("dispatch")?;
    for body in [
        DispatchReply::Accepted,
        DispatchReply::Busy,
        DispatchReply::Rejected,
        DispatchReply::AlreadyCompleted(terminal(&dispatch)?),
        DispatchReply::Terminal(terminal(&dispatch)?),
    ] {
        validate_schema(&dispatch.encode_response(BOOT, WorkerReply::Dispatch(body))?)?;
    }
    let lookup = request("lookup-request")?;
    for body in [
        LookupReply::Unknown,
        LookupReply::Rejected,
        LookupReply::Terminal(terminal(&lookup)?),
    ] {
        validate_schema(&lookup.encode_response(BOOT, WorkerReply::Lookup(body))?)?;
    }
    let ack = request("acknowledge-request")?;
    for body in [
        AcknowledgmentStatus::AlreadyAcknowledged,
        AcknowledgmentStatus::Conflict,
        AcknowledgmentStatus::Rejected,
    ] {
        validate_schema(&ack.encode_response(BOOT, WorkerReply::Acknowledge(body))?)?;
    }
    validate_schema(
        &request("control-request")?
            .encode_response(BOOT, WorkerReply::Control { accepted: false })?,
    )?;
    Ok(())
}

#[test]
fn boot_command_and_terminal_tuple_substitution_fail_closed() -> TestResult {
    let dispatch = request("dispatch")?;
    assert!(
        dispatch
            .encode_response(
                "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
                WorkerReply::Dispatch(DispatchReply::Accepted)
            )
            .is_err()
    );
    assert!(
        dispatch
            .encode_response(BOOT, WorkerReply::Control { accepted: true })
            .is_err()
    );
    let mut forged: Value = serde_json::from_slice(&fixture("dispatch")?)?;
    forged["job_id"] = json!("different-job");
    let other = WorkerRequest::decode(&serde_json::to_vec(&forged)?)?;
    assert!(
        dispatch
            .encode_response(
                BOOT,
                WorkerReply::Dispatch(DispatchReply::Terminal(terminal(&other)?))
            )
            .is_err()
    );
    assert!(terminal(&request("probe-request")?).is_err());
    Ok(())
}

#[test]
fn terminal_digest_binds_all_fifteen_fields_not_only_result_digest() -> TestResult {
    let receipt = terminal(&request("dispatch")?)?;
    let original: Value = serde_json::from_slice(&receipt.encode()?)?;
    let digest = receipt.acknowledgment_digest()?;
    assert_ne!(digest, "9".repeat(64));
    assert_eq!(
        TerminalRecord::decode(&receipt.encode()?)?.acknowledgment_digest()?,
        digest
    );
    for key in original.as_object().ok_or("object")?.keys() {
        let mut changed = original.clone();
        changed[key] = match key.as_str() {
            "payload_digest" => continue, // This version permits only the fixed empty-payload digest.
            "checkpoint_sequence" | "attempt_number" => json!(8),
            "status" => json!("failed"),
            "handoff_id" | "run_id" | "episode_id" | "trajectory_id" => {
                json!("12345678-1234-4234-8234-123456789abc")
            }
            "worker_profile_digest" | "result_digest" => json!("8".repeat(64)),
            _ => json!("different"),
        };
        let changed = TerminalRecord::decode(&serde_json::to_vec(&changed)?)?;
        assert_ne!(changed.acknowledgment_digest()?, digest, "{key}");
    }
    Ok(())
}

#[test]
fn malformed_terminal_rows_cannot_be_encoded_or_acknowledged() -> TestResult {
    let receipt = terminal(&request("dispatch")?)?;
    let original: Value = serde_json::from_slice(&receipt.encode()?)?;
    for key in original.as_object().ok_or("object")?.keys() {
        let mut missing = original.clone();
        missing.as_object_mut().ok_or("object")?.remove(key);
        assert!(
            TerminalRecord::decode(&serde_json::to_vec(&missing)?).is_err(),
            "{key}"
        );
    }
    for (key, value) in [
        ("status", json!("unknown")),
        ("terminal_ref", json!("é".repeat(513))),
        ("checkpoint_sequence", json!(9007199254740992_u64)),
        ("extra", Value::Null),
    ] {
        let mut bad = original.clone();
        bad[key] = value;
        assert!(TerminalRecord::decode(&serde_json::to_vec(&bad)?).is_err());
    }
    let noncanonical = String::from_utf8(receipt.encode()?)?
        .replace("\"checkpoint_sequence\":7", "\"checkpoint_sequence\":7e0");
    assert!(TerminalRecord::decode(noncanonical.as_bytes()).is_err());
    Ok(())
}

#[test]
fn terminal_canonicalization_preserves_unicode_and_required_escapes() -> TestResult {
    let mut value: Value = serde_json::from_slice(&terminal(&request("dispatch")?)?.encode()?)?;
    value["terminal_ref"] = json!("é/\"\\終");
    let bytes = serde_json::to_vec(&value)?;
    let receipt = TerminalRecord::decode(&bytes)?;
    let encoded = String::from_utf8(receipt.encode()?)?;
    assert!(encoded.contains(r#""terminal_ref":"é/\"\\終""#));
    let escaped = String::from_utf8(bytes)?
        .replace('é', "\\u00e9")
        .replace('/', "\\/")
        .replace('終', "\\u7d42");
    let equivalent = TerminalRecord::decode(escaped.as_bytes())?;
    assert_eq!(equivalent.encode()?, receipt.encode()?);
    assert_eq!(
        equivalent.acknowledgment_digest()?,
        receipt.acknowledgment_digest()?
    );
    Ok(())
}

#[test]
fn terminal_input_and_reference_limits_are_exact_byte_bounds() -> TestResult {
    let mut value: Value = serde_json::from_slice(&terminal(&request("dispatch")?)?.encode()?)?;
    value["terminal_ref"] = json!("é".repeat(512));
    let mut bytes = serde_json::to_vec(&value)?;
    assert!(TerminalRecord::decode(&bytes).is_ok());
    bytes.resize(16_384, b' ');
    assert!(TerminalRecord::decode(&bytes).is_ok());
    bytes.push(b' ');
    assert!(TerminalRecord::decode(&bytes).is_err());
    value["terminal_ref"] = json!(format!("{}a", "é".repeat(512)));
    assert!(TerminalRecord::decode(&serde_json::to_vec(&value)?).is_err());
    Ok(())
}
