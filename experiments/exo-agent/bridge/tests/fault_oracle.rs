// SPDX-License-Identifier: MIT

//! Fault matrix and run-isolation oracle for `sts2-harness#148` T2/T3.
//!
//! Every case drives the shipped `sts2-exo-bridge` and the isolated `sts2-exo-executor` against the
//! real pinned Exo TypeScript runtime, replacing only the model service with a synthetic loopback
//! endpoint and the native game with the declared synthetic host. The evidence class is
//! **real-process with a synthetic model and no game** — it is not native-game and not
//! real-provider evidence, and no native effect is claimed.
//!
//! Scope split against `process_oracle.rs`. That oracle already covers the four advertised
//! decisions and input fidelity, escalation and forbidden tools, malformed/oversized/duplicate
//! input and profile rejection, so this file adds only the *admission* fault matrix (schema, pin and
//! identity), a lost model reply, sequential process-restart determinism, and concurrent-run
//! isolation (`#148` T3). It is a separate file rather than an extension of `process_oracle.rs`
//! because that oracle's bytes are pinned by
//! `docs/evidence/exo-executor-process-oracle-20260915.json`.

#[path = "support/fault.rs"]
mod fault;
#[allow(dead_code)]
mod support;

use fault::{FaultModel, Result};
use serde_json::{Value, json};
use std::path::PathBuf;

const PINNED_EXO_REVISION: &str = "b06869ab789dee3f80ca474b5fa89dbe47ccb859";

#[test]
#[ignore = "requires built bridge/executor, pinned Exo source/dependencies and Node; see README"]
fn real_exo_fault_matrix_and_run_isolation() -> Result {
    let root = fault::workspace_root()?;
    let binary = root.join("target/debug/sts2-exo-bridge");
    let envelope = support::projection::request_envelope(&root)?;
    let model = FaultModel::start()?;
    let mut cases = Vec::new();

    admission_faults(&binary, &model, &mut cases)?;
    lost_reply(&binary, &model, &envelope, &mut cases)?;
    restart_determinism(&binary, &envelope, &mut cases)?;
    concurrent_isolation(&binary, &envelope, &mut cases)?;

    support::assert_sources_are_committed(
        &root,
        &[
            "experiments/exo-agent/bridge/tests/fault_oracle.rs",
            "experiments/exo-agent/bridge/tests/support/fault.rs",
        ],
    )?;
    let report = json!({
        "schema": "sts2.exo-one-shot-fault-evidence-v1",
        "evidence": "real-pinned-Exo-with-synthetic-model-no-game",
        "exo_revision": PINNED_EXO_REVISION,
        "harness_revision": String::from_utf8(std::process::Command::new("git")
            .arg("-C").arg(&root).args(["rev-parse", "HEAD"]).output()?.stdout)?.trim(),
        "oracle_sha256": fault::digest(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fault_oracle.rs"))?,
        "support_sha256": fault::digest(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/support/fault.rs"))?,
        "cases": cases,
        "full_runtime_admission": false
    });
    std::fs::write(
        root.join("target/exo-fault-report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}

/// Admission faults must be refused before the executor is spawned, so **zero egress** is the
/// load-bearing assertion here, not merely the error code.
fn admission_faults(
    binary: &std::path::Path,
    model: &FaultModel,
    cases: &mut Vec<Value>,
) -> Result {
    let base = fault::config_json(model)?;
    let mutations: [(&str, &str, fn(&mut Value)); 5] = [
        ("config_schema_mismatch", "exo_bridge_config", |config| {
            config["schema"] = json!("sts2.exo-not-a-schema")
        }),
        (
            "extension_pin_mismatch",
            "exo_bridge_package_identity",
            |config| config["extension_sha256"] = json!("0".repeat(64)),
        ),
        (
            "node_pin_mismatch",
            "exo_bridge_package_identity",
            |config| config["node_sha256"] = json!("0".repeat(64)),
        ),
        (
            "executor_pin_mismatch",
            "exo_bridge_package_identity",
            |config| config["executor_sha256"] = json!("0".repeat(64)),
        ),
        (
            "relative_executor_path",
            "exo_bridge_package_identity",
            |config| config["executor"] = json!("target/exo-executor/debug/sts2-exo-executor"),
        ),
    ];
    for (name, expected, mutate) in mutations {
        let mut config = base.clone();
        mutate(&mut config);
        let path = fault::write_config(&format!("fault-{name}"), &config)?;
        model.reset();
        let output = fault::invoke(binary, "--describe", &path, None, b"", true)?;
        assert!(
            !output.status.success() && output.stdout.is_empty(),
            "{name}"
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            format!("{expected}\n"),
            "{name}"
        );
        assert_eq!(model.request_count(), 0, "{name}");
        cases.push(json!({"case": name, "passed": true, "model_requests": 0,
            "error_code": expected}));
    }

    // The argv digest is the operator-supplied identity of the config file itself, so a digest that
    // does not describe the file is refused before any route or input handling.
    let path = fault::write_config("fault-argv-digest", &base)?;
    let wrong = "0".repeat(64);
    model.reset();
    let output = fault::invoke(binary, "--synthetic", &path, Some(&wrong), b"", true)?;
    assert!(!output.status.success() && output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "exo_bridge_config_identity\n"
    );
    assert_eq!(model.request_count(), 0);
    cases.push(
        json!({"case": "argv_digest_mismatch", "passed": true, "model_requests": 0,
        "error_code": "exo_bridge_config_identity"}),
    );

    // A loopback endpoint is admitted for the synthetic route only: a non-synthetic run must refuse
    // it before any input is read or any process is spawned.
    let path = fault::write_config("fault-provider-route", &base)?;
    model.reset();
    let output = fault::invoke(binary, "--run", &path, None, b"", true)?;
    assert!(!output.status.success() && output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "exo_bridge_provider_route\n"
    );
    assert_eq!(model.request_count(), 0);
    cases.push(
        json!({"case": "provider_route_refusal", "passed": true, "model_requests": 0,
        "error_code": "exo_bridge_provider_route"}),
    );
    Ok(())
}

/// A model endpoint that accepts and consumes the request, then closes with no reply, must make the
/// run fail closed within the bounded process lifetime — never report a decision, and never leak
/// private values. Real Exo still executes, so the failure is the lost reply, not a missing runtime.
fn lost_reply(
    binary: &std::path::Path,
    model: &FaultModel,
    envelope: &Value,
    cases: &mut Vec<Value>,
) -> Result {
    model.drop_replies();
    let config = fault::config_json(model)?;
    let path = fault::write_config("fault-lost-reply", &config)?;
    model.reset();
    let started = std::time::Instant::now();
    let (output, processes) = {
        let temporary = path
            .parent()
            .ok_or("config parent missing")?
            .join("exo-test-tmp");
        std::fs::create_dir_all(&temporary)?;
        let mut command = std::process::Command::new(binary);
        command
            .arg("--synthetic")
            .arg(&path)
            .arg(fault::digest(&path)?)
            .env_clear()
            .env("TMPDIR", temporary);
        fault::run_bounded(&mut command, &serde_json::to_vec(envelope)?, true)?
    };
    let elapsed = started.elapsed();
    model.answer();
    assert!(
        !output.status.success() && output.stdout.is_empty(),
        "lost reply must fail closed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        elapsed < std::time::Duration::from_secs(200),
        "lost reply must terminate within the bounded lifetime: {elapsed:?}"
    );
    assert!(processes >= 3, "real Exo process tree missing: {processes}");
    let requests = model.request_count();
    assert!(
        requests >= 1,
        "lost reply must attempt the model at least once"
    );
    cases.push(
        json!({"case": "lost_reply", "passed": true, "model_requests": requests,
        "elapsed_millis": elapsed.as_millis(), "stdout_empty": true,
        "processes": processes, "evidence": "process-boundary-only"}),
    );
    Ok(())
}

/// Two sequential full process lifecycles of the same request, each with its own private root, must
/// produce the same decision and exactly one model request per run — a poisoned or reused root
/// would show up as a redraw or a divergent decision.
fn restart_determinism(
    binary: &std::path::Path,
    envelope: &Value,
    cases: &mut Vec<Value>,
) -> Result {
    let wait = json!({"decision": "wait", "rationale": "synthetic"});
    let mut decisions = Vec::new();
    let mut requests = Vec::new();
    for run in 0..2 {
        let model = FaultModel::start()?;
        let config = fault::config_json(&model)?;
        let path = fault::write_config(&format!("fault-restart-{run}"), &config)?;
        let output = fault::invoke(
            binary,
            "--synthetic",
            &path,
            None,
            &serde_json::to_vec(envelope)?,
            true,
        )?;
        assert!(
            output.status.success(),
            "restart run {run}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let returned: Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(returned["decision"], wait, "restart run {run}");
        assert_eq!(
            returned["request_id"], envelope["request_id"],
            "restart run {run}"
        );
        assert_eq!(
            returned["turn_id"], envelope["turn_id"],
            "restart run {run}"
        );
        assert_eq!(model.request_count(), 1, "restart run {run}");
        decisions.push(returned["decision"].clone());
        requests.push(model.request_count());
    }
    assert_eq!(decisions[0], decisions[1], "restart must be deterministic");
    cases.push(json!({"case": "restart_determinism", "passed": true,
        "model_requests": requests, "decision": decisions[0]}));
    Ok(())
}

/// `#148` T3: two concurrent runs with separate configs, private `TMPDIR`s and endpoints must not
/// share state — each endpoint sees exactly its own single egress and both decisions are correct.
fn concurrent_isolation(
    binary: &std::path::Path,
    envelope: &Value,
    cases: &mut Vec<Value>,
) -> Result {
    let wait = json!({"decision": "wait", "rationale": "synthetic"});
    let bytes = serde_json::to_vec(envelope)?;
    let handles = ["fault-concurrent-a", "fault-concurrent-b"].map(|name| {
        let binary = binary.to_path_buf();
        let bytes = bytes.clone();
        std::thread::spawn(
            move || -> std::result::Result<(Value, usize, String), String> {
                let run = || -> Result<(Value, usize, String)> {
                    let model = FaultModel::start()?;
                    let config = fault::config_json(&model)?;
                    let path = fault::write_config(name, &config)?;
                    let output = fault::invoke(&binary, "--synthetic", &path, None, &bytes, true)?;
                    if !output.status.success() {
                        return Err(
                            format!("{name}: {}", String::from_utf8_lossy(&output.stderr)).into(),
                        );
                    }
                    let returned: Value = serde_json::from_slice(&output.stdout)?;
                    Ok((returned, model.request_count(), model.endpoint.clone()))
                };
                run().map_err(|error| error.to_string())
            },
        )
    });
    let mut results = Vec::new();
    for handle in handles {
        let (returned, requests, endpoint) =
            handle.join().map_err(|_| "concurrent run panicked")??;
        results.push((returned, requests, endpoint));
    }
    for (returned, requests, _) in &results {
        assert_eq!(returned["decision"], wait);
        assert_eq!(returned["request_id"], envelope["request_id"]);
        assert_eq!(
            *requests, 1,
            "each isolated run sends exactly one model request"
        );
    }
    assert_ne!(
        results[0].2, results[1].2,
        "concurrent runs must not share an endpoint"
    );
    cases.push(json!({"case": "concurrent_isolation", "passed": true,
        "model_requests": [results[0].1, results[1].1],
        "endpoints_distinct": true, "decision": results[0].0["decision"]}));
    Ok(())
}
