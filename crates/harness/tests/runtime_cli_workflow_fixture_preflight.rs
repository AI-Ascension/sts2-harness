// SPDX-License-Identifier: MIT

use std::process::Output;

use serde_json::{Value, json};

#[path = "support/live_workflow_definition.rs"]
mod live_workflow_definition;

#[path = "support/workflow_fixture_process.rs"]
mod workflow_fixture_process;
use workflow_fixture_process::{TestResult, failure, invoke};

const SCHEMA: &str = "console18.r3.workflow-fixture-preflight.v1";
const MARKER: &str = "private-fixture-marker-must-not-be-echoed";
const WORKFLOW_ARGS: &[&str] = &["validate-r3-fixture", "workflow_definition"];
const RESPONSE_KEYS: &[&str] = &[
    "accepted",
    "evidence_class",
    "refusal_code",
    "role",
    "schema",
    "source_sha256",
];

fn valid_bytes() -> TestResult<Vec<u8>> {
    Ok(serde_json::to_vec(&live_workflow_definition::definition()?)?)
}

fn response(output: &Output) -> TestResult<Value> {
    assert!(output.stdout.len() <= 8192);
    assert!(output.stderr.is_empty());
    assert_eq!(
        output.stdout.iter().filter(|byte| **byte == b'\n').count(),
        1
    );
    assert!(output.stdout.ends_with(b"\n"));
    assert!(!output.stdout[..output.stdout.len() - 1].contains(&b' '));
    let value: Value = serde_json::from_slice(&output.stdout)?;
    let object = value
        .as_object()
        .ok_or_else(|| failure("response is not a JSON object"))?;
    let mut keys: Vec<_> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys.as_slice(), RESPONSE_KEYS);
    assert_eq!(value["schema"], SCHEMA);
    assert_eq!(value["role"], "workflow_definition");
    assert_eq!(value["evidence_class"], "source_fixture_validation");
    Ok(value)
}

fn assert_refusal(
    arguments: &[&str],
    input: &[u8],
    code: &str,
    allow_broken_pipe: bool,
) -> TestResult<()> {
    let output = invoke(arguments, input, allow_broken_pipe)?;
    assert_eq!(output.status.code(), Some(2));
    let result = response(&output)?;
    assert_eq!(result["accepted"], false);
    assert_eq!(result["source_sha256"], Value::Null);
    assert_eq!(result["refusal_code"], code);
    assert!(!String::from_utf8_lossy(&output.stdout).contains(MARKER));
    Ok(())
}

fn refuse_workflow(input: &[u8], code: &str) -> TestResult<()> {
    assert_refusal(WORKFLOW_ARGS, input, code, false)
}

#[test]
fn accepts_the_live_strict_fixture_before_runtime_environment_dispatch() -> TestResult<()> {
    let mut input = valid_bytes()?;
    input.extend_from_slice(b"\n \t");
    let output = invoke(
        &["validate-r3-fixture", "workflow_definition"],
        &input,
        false,
    )?;
    assert_eq!(output.status.code(), Some(0));
    let result = response(&output)?;
    assert_eq!(result["accepted"], true);
    assert_eq!(result["source_sha256"], sts2_harness::sha256_hex(&input));
    assert_eq!(result["refusal_code"], Value::Null);
    Ok(())
}

#[test]
fn refuses_malformed_arguments_and_unsupported_roles_in_preflight_mode() -> TestResult<()> {
    for (arguments, code) in [
        (&["validate-r3-fixture"][..], "invalid_arguments"),
        (
            &["validate-r3-fixture", "unknown-role"][..],
            "unsupported_role",
        ),
        (
            &["validate-r3-fixture", "workflow_definition", "extra"][..],
            "invalid_arguments",
        ),
    ] {
        assert_refusal(arguments, b"", code, true)?;
    }
    Ok(())
}

#[test]
fn refuses_duplicate_unknown_float_and_unsafe_integer_using_owner_decoder() -> TestResult<()> {
    let valid = String::from_utf8(valid_bytes()?)?;
    let duplicate = valid.replacen("\"max_steps\":32,", "\"max_steps\":32,\"max_steps\":32,", 1);
    assert_ne!(duplicate, valid);
    refuse_workflow(duplicate.as_bytes(), "invalid_workflow_json")?;

    let unknown = valid.replacen(
        "\"limits\":{",
        &format!("\"limits\":{{\"unknown_{MARKER}\":true,"),
        1,
    );
    assert_ne!(unknown, valid);
    refuse_workflow(unknown.as_bytes(), "invalid_workflow_json")?;

    for (needle, replacement) in [
        ("\"max_steps\":32", "\"max_steps\":32.0"),
        ("\"max_steps\":32", "\"max_steps\":9007199254740992"),
    ] {
        let invalid = valid.replacen(needle, replacement, 1);
        assert_ne!(invalid, valid);
        refuse_workflow(invalid.as_bytes(), "invalid_workflow_json")?;
    }
    Ok(())
}

#[test]
fn refuses_invalid_graph_and_non_r3_profile_after_typed_decode() -> TestResult<()> {
    let mut graph = live_workflow_definition::definition()?;
    graph["graphs"][0]["edges"][0]["to"] = json!("missing-node");
    let graph = serde_json::to_vec(&graph)?;
    refuse_workflow(&graph, "invalid_workflow_definition")?;

    let mut profile = live_workflow_definition::definition()?;
    profile["game_profile"] = json!("synthetic-sts2-v1");
    let profile = serde_json::to_vec(&profile)?;
    refuse_workflow(&profile, "invalid_workflow_definition")?;
    Ok(())
}

#[test]
fn refuses_input_over_the_bounded_stdin_limit() -> TestResult<()> {
    let input = vec![b' '; 65_537];
    refuse_workflow(&input, "input_too_large")?;
    Ok(())
}
