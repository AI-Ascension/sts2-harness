// SPDX-License-Identifier: MIT

mod support;

use serde_json::{Value, json};
use std::path::PathBuf;
use support::{Model, Result, digest, invoke, response};

/// Explicitly selected Linux-only real Exo test, using only a synthetic loopback model.
#[test]
#[ignore = "requires built bridge, pinned Exo source/dependencies and Node; see README"]
fn real_exo_process_matrix() -> Result {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()?;
    let binary = root.join("target/debug/sts2-exo-bridge");
    let executor = root.join("target/exo-executor/debug/sts2-exo-executor");
    let source = root.join("target/exo-source");
    let node = PathBuf::from(std::env::var("STS2_EXO_TEST_NODE")?).canonicalize()?;
    let extension = source.join("experiments/exo-agent/extension/src/index.ts");
    assert_eq!(
        digest(&extension)?,
        digest(&root.join("experiments/exo-agent/extension/src/index.ts"))?
    );
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
    decisions(&model, &binary, &config, &envelope, &mut cases)?;
    failed_process_boundaries(&model, &binary, &config, &envelope, &mut cases)?;
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
        "cases": cases, "full_runtime_admission": false
    });
    std::fs::write(
        root.join("target/exo-smoke-report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}

fn request_envelope(root: &std::path::Path) -> Result<Value> {
    let mut request: Value = serde_json::from_slice(&std::fs::read(
        root.join("protocol-artifact/exo-bridge-v1/golden/request.json"),
    )?)?;
    request["objective"] = json!("synthetic exact objective sentinel");
    request["hard_constraints"] = json!(["synthetic complete constraint sentinel"]);
    Ok(json!({
        "wire_version": "sts2.exo-bridge-wire-v1",
        "request_id": "host-request-private-sentinel",
        "turn_id": "host-turn-private-sentinel", "request": request
    }))
}

fn failed_process_boundaries(
    model: &Model,
    binary: &std::path::Path,
    config: &std::path::Path,
    envelope: &Value,
    cases: &mut Vec<Value>,
) -> Result {
    let mut tampered: Value = serde_json::from_slice(&std::fs::read(config)?)?;
    tampered["executor_sha256"] = json!("0".repeat(64));
    std::fs::write(config, serde_json::to_vec(&tampered)?)?;
    model.set(200, response("{}", "message"))?;
    let denied = invoke(binary, config, b"", "--describe", true)?;
    assert!(!denied.status.success() && denied.stdout.is_empty());
    assert!(model.requests.lock().map_err(|_| "poisoned")?.is_empty());
    assert_eq!(model.request_count(), 0);
    cases.push(json!({"case": "package_digest_mismatch", "passed": true, "model_requests": 0}));
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
        let diagnostics = String::from_utf8(output.stderr)?;
        let evidence = diagnostics
            .lines()
            .filter(|line| line.starts_with('{'))
            .map(serde_json::from_str::<Value>)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        assert_eq!(evidence.len(), 1, "{name}: {diagnostics}");
        let row = &evidence[0];
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

fn projection(body: &Value, envelope: &Value) -> Result {
    assert!(body.get("tools").is_none_or(|tools| tools == &json!([])));
    let serialized = body.to_string();
    for field in ["request_id", "turn_id"] {
        assert!(!serialized.contains(envelope[field].as_str().ok_or("missing host id")?));
    }
    assert!(
        !serialized.contains(
            envelope["request"]["model_execution_id"]
                .as_str()
                .ok_or("missing execution id")?
        )
    );
    fn strings<'a>(value: &'a Value, values: &mut Vec<&'a str>) {
        match value {
            Value::String(text) => values.push(text),
            Value::Array(items) => items.iter().for_each(|item| strings(item, values)),
            Value::Object(items) => items.values().for_each(|item| strings(item, values)),
            _ => {}
        }
    }
    let mut values = Vec::new();
    strings(body, &mut values);
    let projections = values
        .into_iter()
        .filter_map(|text| serde_json::from_str::<Value>(text).ok())
        .filter(|value| value.get("observation").is_some())
        .collect::<Vec<_>>();
    assert_eq!(projections.len(), 1);
    for key in [
        "observation",
        "legal_action_ids",
        "objective",
        "hard_constraints",
    ] {
        assert_eq!(projections[0][key], envelope["request"][key], "{key}");
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
            serde_json::to_vec(&support::ordinary_map(envelope)?)?,
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
