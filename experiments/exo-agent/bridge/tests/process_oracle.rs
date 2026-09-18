// SPDX-License-Identifier: MIT

mod support;

use serde_json::{Value, json};
use std::path::PathBuf;
use support::projection::{FORBIDDEN_TOOLS, evidence_row, projection, request_envelope};
use support::{Model, Result, digest, invoke, invoke_executor, response, tool_call};

/// Explicitly selected Linux-only real Exo test, using only a synthetic loopback model.
#[test]
#[ignore = "requires built bridge, pinned Exo source/dependencies and Node; see README"]
fn real_exo_process_matrix() -> Result {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()?;
    let binary = root.join("target/debug/sts2-exo-bridge");
    let executor = root.join("target/exo-executor/debug/sts2-exo-executor");
    let source = std::env::var_os("STS2_EXO_TEST_SOURCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target/exo-source"));
    let node = PathBuf::from(std::env::var("STS2_EXO_TEST_NODE")?).canonicalize()?;
    let extension = root.join("experiments/exo-agent/extension/src/index.ts");
    let model = Model::start()?;
    let config = root.join("target/exo-oracle-config.json");
    std::fs::write(
        &config,
        serde_json::to_vec(&json!({
            "schema": "sts2.exo-one-shot-config-v1",
            "executor": executor, "executor_sha256": digest(&executor)?,
            "source_root": source, "extension": extension, "extension_sha256": digest(&extension)?,
            "node": node, "node_sha256": digest(&node)?,
            "model": "o3-pro", "endpoint": model.endpoint
        }))?,
    )?;
    let envelope = request_envelope(&root)?;
    let mut cases = Vec::new();
    let described = invoke(&binary, &config, b"", "--describe", true)?;
    assert!(described.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&described.stdout)?["full_runtime_admission"],
        false
    );
    assert!(model.requests.lock().map_err(|_| "poisoned")?.is_empty());
    assert_eq!(model.request_count(), 0);
    cases.push(json!({"case": "describe", "passed": true, "model_requests": 0}));
    rejected_inputs(&model, &binary, &config, &envelope, &mut cases)?;
    failed_process_boundaries(&model, &binary, &config, &envelope, &mut cases)?;
    decisions(&model, &binary, &config, &envelope, &mut cases)?;
    request_tools_are_empty(&model, &binary, &config, &envelope, &mut cases)?;
    forbidden_tools(&model, &binary, &config, &envelope, &mut cases)?;
    std::fs::remove_file(config)?;
    let report = json!({
        "schema": "sts2.exo-one-shot-process-evidence-v1",
        "evidence": "real-pinned-Exo-with-synthetic-model-no-game",
        "temporary_storage": "owned-target-exo-test-tmp",
        "bridge_sha256": digest(&binary)?, "executor_sha256": digest(&executor)?,
        "extension_sha256": digest(&extension)?,
        "exo_revision": "b06869ab789dee3f80ca474b5fa89dbe47ccb859",
        "harness_revision": String::from_utf8(std::process::Command::new("git")
            .arg("-C").arg(&root).args(["rev-parse", "HEAD"]).output()?.stdout)?.trim(),
        "oracle_sha256": digest(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/process_oracle.rs"))?,
        "support_sha256": support::support_digests()?,
        "cases": cases, "full_runtime_admission": false
    });
    std::fs::write(
        root.join("target/exo-smoke-report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}

fn failed_process_boundaries(
    model: &Model,
    binary: &std::path::Path,
    config: &std::path::Path,
    envelope: &Value,
    cases: &mut Vec<Value>,
) -> Result {
    let original = std::fs::read(config)?;
    let mut tampered: Value = serde_json::from_slice(&original)?;
    let digest_config = config.with_file_name("exo-oracle-digest-config.json");
    let nonzero_config = config.with_file_name("exo-oracle-nonzero-config.json");
    let config = digest_config.as_path();
    tampered["executor_sha256"] = json!("0".repeat(64));
    std::fs::write(config, serde_json::to_vec(&tampered)?)?;
    model.set(200, response("{}", "message"))?;
    let denied = invoke(binary, config, b"", "--describe", true)?;
    assert!(!denied.status.success() && denied.stdout.is_empty());
    assert!(model.requests.lock().map_err(|_| "poisoned")?.is_empty());
    assert_eq!(model.request_count(), 0);
    cases.push(json!({"case": "package_digest_mismatch", "passed": true, "model_requests": 0}));
    std::fs::remove_file(config)?;
    let config = nonzero_config.as_path();
    let mut tampered: Value = serde_json::from_slice(&original)?;
    tampered["executor"] = json!("/usr/bin/false");
    tampered["executor_sha256"] = json!(digest(std::path::Path::new("/usr/bin/false"))?);
    std::fs::write(config, serde_json::to_vec(&tampered)?)?;
    let failed = invoke(
        binary,
        config,
        &serde_json::to_vec(envelope)?,
        "--synthetic",
        true,
    )?;
    assert!(!failed.status.success() && failed.stdout.is_empty());
    assert!(model.requests.lock().map_err(|_| "poisoned")?.is_empty());
    assert_eq!(model.request_count(), 0);
    cases.push(
        json!({"case": "executor_nonzero", "passed": true, "model_requests": 0,
        "evidence": "process-boundary-only"}),
    );
    std::fs::remove_file(config)?;
    Ok(())
}

fn decisions(
    model: &Model,
    binary: &std::path::Path,
    config: &std::path::Path,
    envelope: &Value,
    cases: &mut Vec<Value>,
) -> Result {
    let action = &envelope["request"]["legal_action_ids"][0];
    let wait = json!({"decision": "wait", "rationale": "synthetic"});
    for (name, decision, success, status, kind) in [
        (
            "action",
            json!({"decision": "action", "action_id": action, "rationale": "synthetic"}),
            true,
            200,
            "message",
        ),
        (
            "plan",
            json!({"decision": "plan", "action_ids": [action], "rationale": "synthetic"}),
            true,
            200,
            "message",
        ),
        ("wait", wait.clone(), true, 200, "message"),
        (
            "reobserve",
            json!({"decision": "reobserve", "rationale": "synthetic"}),
            true,
            200,
            "message",
        ),
        (
            "illegal_action",
            json!({"decision": "action", "action_id": "invented", "rationale": "synthetic"}),
            false,
            200,
            "message",
        ),
        ("multiple_json", json!("{}{}"), false, 200, "message"),
        (
            "truncated_json",
            json!("{\"decision\":"),
            false,
            200,
            "message",
        ),
        ("empty_output", json!(""), false, 200, "message"),
        (
            "oversized_output",
            json!("x".repeat(8193)),
            false,
            200,
            "message",
        ),
        (
            "unknown_field",
            json!({"decision": "wait", "rationale": "synthetic", "unknown": true}),
            false,
            200,
            "message",
        ),
        ("refusal", json!({}), false, 200, "refusal"),
        ("tool_escalation", json!({}), false, 200, "tool"),
        ("multiple_messages", wait, false, 200, "multiple"),
        ("429_one_egress", json!({}), false, 429, "message"),
        ("500_one_egress", json!({}), false, 500, "message"),
    ] {
        let text = decision
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| decision.to_string());
        model.set(status, response(&text, kind))?;
        let started = std::time::Instant::now();
        let output = invoke(
            binary,
            config,
            &serde_json::to_vec(envelope)?,
            "--synthetic",
            true,
        )?;
        eprintln!(
            "case {name}: {:?}, status {}",
            started.elapsed(),
            output.status
        );
        assert_eq!(
            output.status.success(),
            success,
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let observed = model.requests.lock().map_err(|_| "poisoned")?;
        assert_eq!(observed.len(), 1, "{name}");
        assert_eq!(model.request_count(), 1, "{name}");
        projection(&observed[0], envelope)?;
        let row = evidence_row(&output.stderr, name)?;
        assert_eq!(row["forwarded_requests"], 1, "{name}");
        assert_eq!(
            row["exo_turn_id"].as_str().ok_or("missing id")?.as_bytes()[14],
            b'7'
        );
        if status != 200 {
            assert!(row["denied_requests"].as_u64().ok_or("missing count")? >= 1);
        }
        if success {
            let returned: Value = serde_json::from_slice(&output.stdout)?;
            assert_eq!(returned["decision"], decision);
            assert_eq!(returned["request_id"], envelope["request_id"]);
            assert_eq!(returned["turn_id"], envelope["turn_id"]);
        } else {
            assert!(output.stdout.is_empty(), "{name}");
        }
        cases.push(json!({"case": name, "passed": true, "model_requests": 1,
            "argv_private_values_absent": true, "evidence": row}));
    }
    Ok(())
}

// Named case for the inline `tools` check: the actual request advertises no tool or tool choice.
fn request_tools_are_empty(
    model: &Model,
    binary: &std::path::Path,
    config: &std::path::Path,
    envelope: &Value,
    cases: &mut Vec<Value>,
) -> Result {
    model.set(
        200,
        response(
            "{\"decision\":\"wait\",\"rationale\":\"synthetic\"}",
            "message",
        ),
    )?;
    let output = invoke(
        binary,
        config,
        &serde_json::to_vec(envelope)?,
        "--synthetic",
        true,
    )?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let observed = model.requests.lock().map_err(|_| "poisoned")?;
    assert_eq!((observed.len(), model.request_count()), (1, 1));
    let advertised = observed[0]
        .get("tools")
        .cloned()
        .unwrap_or_else(|| json!([]));
    assert_eq!(advertised, json!([]));
    assert!(observed[0].get("tool_choice").is_none());
    projection(&observed[0], envelope)?;
    let row = evidence_row(&output.stderr, "request_tools_are_empty")?;
    cases.push(
        json!({"case": "request_tools_are_empty", "passed": true, "model_requests": 1,
        "tools_advertised": 0, "argv_private_values_absent": true, "evidence": row}),
    );
    Ok(())
}

// Per forbidden name the synthetic model calls that exact tool: the bridge fails closed after one
// egress and the executor receipt carries the typed `exo_forbidden_tool` code and no decision.
fn forbidden_tools(
    model: &Model,
    binary: &std::path::Path,
    config: &std::path::Path,
    envelope: &Value,
    cases: &mut Vec<Value>,
) -> Result {
    for (ordinal, (alias, name)) in FORBIDDEN_TOOLS.into_iter().enumerate() {
        let case = format!("forbidden_tool_by_name_{alias}");
        model.set(200, tool_call(name))?;
        let output = invoke(
            binary,
            config,
            &serde_json::to_vec(envelope)?,
            "--synthetic",
            true,
        )?;
        assert!(
            !output.status.success() && output.stdout.is_empty(),
            "{case}"
        );
        let stderr = String::from_utf8(output.stderr)?;
        assert!(
            stderr.ends_with("exo_bridge_executor_failed\n"),
            "{case}: {stderr}"
        );
        let bridge = evidence_row(stderr.as_bytes(), &case)?;
        assert_eq!(
            (bridge["forwarded_requests"].as_u64(), model.request_count()),
            (Some(1), 1)
        );
        projection(&model.requests.lock().map_err(|_| "poisoned")?[0], envelope)?;
        model.set(200, tool_call(name))?;
        let receipt = invoke_executor(config, envelope, ordinal)?;
        assert_eq!(
            receipt["error_code"], "exo_forbidden_tool",
            "{case}: {receipt}"
        );
        assert!(receipt["decision"].is_null(), "{case}");
        assert_eq!(receipt["request_id"], envelope["request_id"], "{case}");
        assert_eq!(
            (
                receipt["forwarded_requests"].as_u64(),
                model.request_count()
            ),
            (Some(1), 1)
        );
        projection(&model.requests.lock().map_err(|_| "poisoned")?[0], envelope)?;
        cases.push(
            json!({"case": case, "passed": true, "model_requests": 2, "tool_name": name,
            "argv_private_values_absent": true, "evidence": {"bridge": bridge,
            "bridge_error_code": "exo_bridge_executor_failed",
            "executor_error_code": "exo_forbidden_tool", "executor_decision_absent": true}}),
        );
    }
    Ok(())
}

fn rejected_inputs(
    model: &Model,
    binary: &std::path::Path,
    config: &std::path::Path,
    envelope: &Value,
    cases: &mut Vec<Value>,
) -> Result {
    let mut revision = envelope.clone();
    revision["request"]["provider_revision"] = json!("f".repeat(40));
    let mut generation = envelope.clone();
    generation["request"]["generation"] = json!(999);
    let mut unknown = envelope.clone();
    unknown["request"]["unknown"] = json!(true);
    let mut expert = envelope.clone();
    expert["request"]["observation"]["protocol_version"] = json!("runtime-v4-expert");
    for (name, bytes, eof) in [
        ("wrong_revision", serde_json::to_vec(&revision)?, true),
        (
            "unsupported_map",
            serde_json::to_vec(&support::projection::ordinary_map(envelope)?)?,
            true,
        ),
        ("wrong_generation", serde_json::to_vec(&generation)?, true),
        ("unknown_field_input", serde_json::to_vec(&unknown)?, true),
        ("unsupported_expert", serde_json::to_vec(&expert)?, true),
        ("invalid_utf8", vec![255], true),
        ("oversized_input", vec![b'x'; 131073], true),
        (
            "duplicate_field_input",
            br#"{"wire_version":"x","wire_version":"y"}"#.to_vec(),
            true,
        ),
        ("missing_eof", serde_json::to_vec(envelope)?, false),
    ] {
        model.set(200, response("{}", "message"))?;
        let result = invoke(binary, config, &bytes, "--synthetic", eof)?;
        assert!(
            !result.status.success() && result.stdout.is_empty(),
            "{name}"
        );
        if name == "unsupported_map" {
            assert_eq!(result.stderr, b"exo_bridge_unsupported_profile\n");
        }
        assert!(
            model.requests.lock().map_err(|_| "poisoned")?.is_empty(),
            "{name}"
        );
        assert_eq!(model.request_count(), 0, "{name}");
        cases.push(json!({"case": name, "passed": true, "model_requests": 0}));
    }
    Ok(())
}
