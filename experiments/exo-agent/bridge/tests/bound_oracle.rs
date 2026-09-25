// SPDX-License-Identifier: MIT

//! Writer-side back-pressure and turn-budget oracle for `sts2-harness#148`.
//!
//! Every case drives the shipped `sts2-exo-bridge` or the isolated `sts2-exo-executor` against the
//! real pinned Exo TypeScript runtime, replacing only the model service with a synthetic loopback
//! endpoint. The evidence class is **real-process with a synthetic model and no game**: it is not
//! native-game and not real-provider evidence, and no native effect is claimed. The cases that
//! drive the executor directly pass a synthetic `input` object, so a passing case proves the bound
//! *process* honours its own contract; it does not prove a bridge-issued handoff can approach
//! 160 KiB, and one case below measures why it cannot.
//!
//! Scope split against the two oracles already in this lane. `process_oracle.rs` writes 131,073
//! bytes and then EOF (`oversized_input`), so it covers the bridge's **read/parse** bound — the
//! count it accepts at all. `fault_oracle.rs` covers admission faults and run isolation. Neither
//! covers back-pressure (how much a writer gets into a peer that has already stopped reading) nor
//! the executor's turn budget. Both are `#[ignore]`d process oracles, so this file joins them
//! rather than extending a file whose bytes the 20260917 evidence record pins.

#[path = "support/bounds.rs"]
mod bounds;

use bounds::{Loopback, Result};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

const EXECUTOR_INPUT_LIMIT: usize = 160 * 1024;
const BRIDGE_REQUEST_BOUND: usize = 131_072;

#[test]
#[ignore = "requires built bridge/executor, pinned Exo source/dependencies and Node; see README"]
fn real_exo_back_pressure_and_budget_exhaustion() -> Result {
    let root = bounds::workspace_root()?;
    let bridge = root.join("target/debug/sts2-exo-bridge");
    let executor = root.join("target/exo-executor/debug/sts2-exo-executor");
    let envelope = bounds::request_envelope(&root)?;
    let model = Loopback::start()?;
    let config = bounds::write_config(&root, "bound-oracle", &bounds::config_json(&root, &model)?)?;
    let digest = bounds::digest(&config)?;
    let mut cases = Vec::new();

    bridge_read_bound_is_not_back_pressure(&bridge, &config, &digest, &model, &mut cases)?;
    executor_stops_reading_at_its_own_bound(&root, &config, &envelope, &mut cases)?;
    executor_budget_exhaustion(&root, &config, &envelope, &model, &mut cases)?;

    std::fs::remove_file(&config)?;
    // The report names `HEAD`; refuse to emit it unless `HEAD` really carries the recorded bytes.
    bounds::assert_sources_are_committed(
        &root,
        &[
            "experiments/exo-agent/bridge/tests/bound_oracle.rs",
            "experiments/exo-agent/bridge/tests/support/bounds.rs",
            "experiments/exo-agent/bridge/tests/support/loopback.rs",
        ],
    )?;
    let report = json!({
        "schema": "sts2.exo-one-shot-bound-evidence-v1",
        "evidence": "real-pinned-Exo-with-synthetic-model-no-game",
        "exo_revision": bounds::driven_exo_revision(&root, &bounds::source_root(&root)?)?,
        "harness_revision": String::from_utf8(std::process::Command::new("git")
            .arg("-C").arg(&root).args(["rev-parse", "HEAD"]).output()?.stdout)?.trim(),
        "bridge_sha256": bounds::digest(&bridge)?,
        "executor_sha256": bounds::digest(&executor)?,
        "oracle_sha256": bounds::digest(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/bound_oracle.rs"))?,
        "support_sha256": bounds::digest(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/support/bounds.rs"))?,
        "loopback_support_sha256": bounds::digest(
            &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/support/loopback.rs"))?,
        "cases": cases,
        "boundaries": {
            "bridge_request_parse_bound_bytes": BRIDGE_REQUEST_BOUND,
            "executor_read_bound_bytes": EXECUTOR_INPUT_LIMIT,
            "extension_model_write_bound_bytes": EXECUTOR_INPUT_LIMIT,
            "executor_read_bound_reachable_through_the_bridge": false,
            "note": "The executor read bound admits a 160 KiB handoff, but the turn is then denied \
                     locally by the extension's equal model-write bound before any inference, so the \
                     read bound is reachable only by a direct drive and does not itself yield a \
                     decision."
        },
        "full_runtime_admission": false
    });
    std::fs::write(
        root.join("target/exo-bound-report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    Ok(())
}

/// Both boundaries are reachable and they are *different* boundaries.
///
/// The bridge reads at most 131,072 bytes, so a writer offering one byte past that is stopped by the
/// bridge's own parse refusal while the executor — reached only after a valid envelope — is stopped
/// by its 160 KiB read bound. The byte count the writer got in is reported, not asserted exactly:
/// it is a property of the kernel pipe buffer as much as of the process.
///
/// The two offers pin the read count itself: 131,072 bytes are the most the bridge will read (and
/// still fail to parse, because the payload is not an envelope), and 131,073 is one byte over the
/// `take` bound. The process count is recorded rather than asserted, because the bridge's config
/// load spawns transient `git` children (`rev-parse`, `status`) that a single sample can catch. The
/// pre-inference claim is carried by the model's own connection count, which is exact: no executor
/// and no model request is reached.
fn bridge_read_bound_is_not_back_pressure(
    bridge: &Path,
    config: &Path,
    digest: &str,
    model: &Loopback,
    cases: &mut Vec<Value>,
) -> Result {
    for (name, offered) in [
        ("bridge_offer_at_parse_bound", BRIDGE_REQUEST_BOUND),
        ("bridge_offer_past_parse_bound", BRIDGE_REQUEST_BOUND + 1),
    ] {
        let mut command = bounds::bridge_command(bridge, config, digest)?;
        let run = bounds::run_offering(&mut command, &vec![b'x'; offered])?;
        assert!(
            !run.output.status.success() && run.output.stdout.is_empty(),
            "{name}: {}",
            run.stderr_line()
        );
        assert_eq!(run.stderr_line(), "exo_bridge_invalid_request", "{name}");
        // The parse refusal is pre-inference: no executor and no model is reached. This is the
        // exact half of the claim; `processes` is recorded only, for the reason in the doc above.
        assert_eq!(
            model.request_count(),
            0,
            "{name} reached the model endpoint"
        );
        cases.push(json!({"case": name, "passed": true, "offered": offered,
            "written": run.written, "processes": run.processes,
            "model_requests": model.request_count(),
            "error_code": "exo_bridge_invalid_request",
            "evidence": "bridge-parse-refusal-pre-spawn"}));
    }

    // A bridge-issued handoff cannot approach the executor's read bound: the projection the bridge
    // builds carries at most 32 constraints, so its parse bound always fires first. This records
    // that measured relationship instead of implying the executor bound is unreachable through the
    // bridge.
    let mut saturated = bounds::request_envelope(&bounds::workspace_root()?)?;
    saturated["request"]["hard_constraints"] = json!(
        (0..32)
            .map(|index| format!("{index:03}{}", "x".repeat(508)))
            .collect::<Vec<_>>()
    );
    let projected = serde_json::to_vec(&json!({
        "observation": saturated["request"]["observation"],
        "legal_action_ids": saturated["request"]["legal_action_ids"],
        "objective": saturated["request"]["objective"],
        "hard_constraints": saturated["request"]["hard_constraints"]
    }))?;
    assert!(
        projected.len() < EXECUTOR_INPUT_LIMIT / 4,
        "a saturated bridge projection must stay far below the executor bound: {}",
        projected.len()
    );
    cases.push(
        json!({"case": "bridge_projection_cannot_reach_executor_bound", "passed": true,
        "saturated_projection_bytes": projected.len(),
        "executor_input_limit": EXECUTOR_INPUT_LIMIT,
        "bridge_request_bound": BRIDGE_REQUEST_BOUND,
        "model_requests": 0, "evidence": "measured-at-maximum-constraints"}),
    );
    Ok(())
}

/// The executor stops reading at its own bound and kills the pipe, so a writer offering far more is
/// stopped at roughly `INPUT_LIMIT` — an EPIPE rather than a silent truncation.
///
/// Three padded sizes pin the boundary, and the middle one is the reason the case is written this
/// way rather than as a single "at the bound, the turn completes" claim. At `INPUT_LIMIT` the executor
/// reads the whole handoff and admits it, but the turn then fails **locally in the extension's own
/// model-write guard**: the projected request body is larger than the extension's 160 KiB bound, so
/// it denies three SDK attempts before any inference and the endpoint sees zero requests. This lane
/// therefore reports the read bound as reachable-but-not-sufficient, instead of implying that a
/// handoff at 160 KiB yields a decision. One size below (`INPUT_LIMIT - 4 KiB`) completes a real
/// turn, so the refusal at the bound is the projection guard and not a broken drive.
fn executor_stops_reading_at_its_own_bound(
    root: &Path,
    config: &Path,
    envelope: &Value,
    cases: &mut Vec<Value>,
) -> Result {
    let model = Loopback::start()?;
    let bounded = bounds::write_config(root, "bound-limit", &bounds::config_json(root, &model)?)?;
    // The handoff is built from the same config it is driven with. The executor admits only the
    // endpoint named by its own `STS2_EXO_ALLOWED_ENDPOINT`, so a handoff built from a *different*
    // loopback would be refused at the binding and the at-bound case would prove the wrong thing.
    // The `state_root` here is the template each case rebases; no drive ever writes it.
    let handoff = bounds::executor_handoff(
        &bounded,
        envelope,
        &root.join("target/exo-test-tmp/bound-handoff-template"),
        115_000,
        4096,
    )?;
    model.reset()?;
    for (name, target, outcome) in [
        (
            "executor_handoff_below_projection_bound",
            EXECUTOR_INPUT_LIMIT - 4 * 1024,
            "completed",
        ),
        (
            "executor_handoff_at_input_limit_read_admitted",
            EXECUTOR_INPUT_LIMIT,
            "projection_denied",
        ),
        (
            "executor_handoff_past_input_limit",
            EXECUTOR_INPUT_LIMIT + 1,
            "read_refused",
        ),
    ] {
        // Each drive owns a fresh private root. Exo refuses a second conversation in a store an
        // earlier drive already owns (`exo_executor_agent`), and the padding is applied *after* the
        // rebase, so a per-case root keeps the exact byte count the case pins.
        let case_root = bounds::executor_root(root, name)?;
        let payload = bounds::handoff_at_size(&handoff, target, Some(&case_root))?;
        let mut command = bounds::executor_command(&bounded, &case_root)?;
        model.reset()?;
        let run = bounds::run_offering(&mut command, &payload)?;
        std::fs::remove_dir_all(&case_root)?;
        match outcome {
            "completed" => {
                assert!(
                    run.output.status.success(),
                    "{name} is below the projection bound and must complete: {}",
                    run.stderr_line()
                );
                let receipt: Value = serde_json::from_slice(&run.output.stdout)?;
                assert_eq!(receipt["version"], "sts2.exo-executor-receipt-v2");
                assert_ne!(
                    receipt["decision"],
                    Value::Null,
                    "{name} must complete a real turn, not merely avoid the read bound"
                );
                assert_eq!(receipt["error_code"], Value::Null, "{name}");
                assert_eq!(receipt["fetch_attempts"], 1, "{name}");
                assert_eq!(receipt["forwarded_requests"], 1, "{name}");
                assert_eq!(receipt["denied_requests"], 0, "{name}");
                assert_eq!(
                    model.request_count(),
                    1,
                    "{name} must reach the endpoint once"
                );
            }
            "projection_denied" => {
                // The read bound admits this handoff; the turn is then refused by the extension's
                // 160 KiB model-write guard. Both halves are asserted, so neither a tightened read
                // bound (which would print the typed read refusal) nor a weakened projection guard
                // (which would let a request reach the endpoint and return a decision) passes.
                assert!(
                    run.output.status.success(),
                    "{name} is inside the read bound and must be admitted by it: {}",
                    run.stderr_line()
                );
                let receipt: Value = serde_json::from_slice(&run.output.stdout)?;
                assert_eq!(receipt["version"], "sts2.exo-executor-receipt-v2");
                assert_eq!(receipt["decision"], Value::Null, "{name}");
                assert_eq!(receipt["error_code"], "exo_turn_failed", "{name}");
                assert_eq!(receipt["forwarded_requests"], 0, "{name}");
                assert_eq!(
                    receipt["denied_requests"], receipt["fetch_attempts"],
                    "{name}"
                );
                assert_eq!(
                    model.request_count(),
                    0,
                    "{name} must be denied locally, before the endpoint"
                );
            }
            _ => {
                assert!(!run.output.status.success(), "{name}");
                assert_eq!(run.stderr_line(), "exo_executor_input_bound", "{name}");
                assert_eq!(model.request_count(), 0, "{name}");
            }
        }
        cases.push(json!({"case": name, "passed": true, "bytes": target,
            "outcome": outcome,
            "written": run.written, "processes": run.processes,
            "error_line": run.stderr_line(),
            "model_requests": model.request_count(),
            "evidence": "executor-read-bound"}));
    }
    std::fs::remove_file(&bounded)?;

    // Back-pressure proper: offer far more than the bound without ever sending EOF first. The
    // executor reads INPUT_LIMIT + 1 bytes, refuses the input and exits, so the writer is stopped
    // at the bound instead of the pipe swallowing the whole offer.
    let offered = 8 * EXECUTOR_INPUT_LIMIT;
    let run =
        bounds::run_executor_bytes(root, config, "executor-back-pressure", &vec![b'x'; offered])?;
    assert!(
        !run.output.status.success() && run.output.stdout.is_empty(),
        "back-pressure run must fail closed: {}",
        run.stderr_line()
    );
    assert_eq!(run.stderr_line(), "exo_executor_input_bound");
    assert!(
        run.written < offered,
        "the writer offered {offered} bytes and got all of them in; that is not back-pressure"
    );
    assert!(
        run.stopped_between(EXECUTOR_INPUT_LIMIT, EXECUTOR_INPUT_LIMIT + 512 * 1024),
        "the executor stopped the writer at {} bytes, outside the expected band",
        run.written
    );
    cases.push(
        json!({"case": "executor_writer_stops_at_input_bound", "passed": true,
        "offered": offered, "written": run.written,
        "write_error": format!("{:?}", run.write_error),
        "input_limit": EXECUTOR_INPUT_LIMIT, "processes": run.processes,
        "error_code": "exo_executor_input_bound",
        "evidence": "writer-side-back-pressure-counted"}),
    );
    Ok(())
}

/// The two declared turn budgets bound the turn in ways the existing suite does not cover.
///
/// `timeout_millis` is a real deadline, not a validation-only field: with the synthetic endpoint
/// holding the reply outstanding, the executor aborts mid-turn and reports the typed timeout.
/// `max_output_tokens` reaches the model service, and a reply the provider truncated at that budget
/// yields no decision rather than a fabricated one.
fn executor_budget_exhaustion(
    root: &Path,
    config: &Path,
    envelope: &Value,
    model: &Loopback,
    cases: &mut Vec<Value>,
) -> Result {
    model.reset()?;
    model.set_reply(bounds::one_shot_wait(), 30_000)?;
    let started = std::time::Instant::now();
    let run = bounds::run_executor(root, config, envelope, "mid-turn-timeout", 10_000, 4096)?;
    let elapsed = started.elapsed();
    assert!(
        !run.output.status.success() && run.output.stdout.is_empty(),
        "mid-turn timeout: {}",
        run.stderr_line()
    );
    assert_eq!(run.stderr_line(), "exo_executor_turn_timeout");
    assert!(
        elapsed >= std::time::Duration::from_secs(10),
        "the declared 10 s deadline fired early: {elapsed:?}"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(60),
        "the declared 10 s deadline did not bound the turn: {elapsed:?}"
    );
    assert!(
        run.processes >= 2,
        "real Exo process tree missing: {}",
        run.processes
    );
    cases.push(json!({"case": "executor_mid_turn_deadline", "passed": true,
        "timeout_millis": 10_000, "elapsed_millis": elapsed.as_millis(),
        "model_requests_offered": model.request_count(), "processes": run.processes,
        "error_code": "exo_executor_turn_timeout", "deadline_is_measured": true}));

    model.reset()?;
    model.set_reply(bounds::truncated_at_budget(), 0)?;
    let run = bounds::run_executor(root, config, envelope, "truncated-at-budget", 115_000, 1024)?;
    assert!(
        run.output.status.success(),
        "a truncated reply is a failed turn, not a process failure: {}",
        run.stderr_line()
    );
    let receipt: Value = serde_json::from_slice(&run.output.stdout)?;
    assert_eq!(receipt["version"], "sts2.exo-executor-receipt-v2");
    assert_eq!(receipt["decision"], Value::Null);
    assert_eq!(receipt["error_code"], "exo_turn_failed");
    assert_eq!(receipt["fetch_attempts"], 1);
    assert_eq!(receipt["forwarded_requests"], 1);
    assert_eq!(receipt["denied_requests"], 0);
    let bodies = model.bodies()?;
    assert_eq!(bodies.len(), 1, "exactly one model request");
    assert_eq!(bodies[0]["max_output_tokens"], 1024);
    assert_eq!(bodies[0]["stream"], false);
    cases.push(
        json!({"case": "executor_truncated_at_output_budget", "passed": true,
        "max_output_tokens": 1024, "wire_max_output_tokens": bodies[0]["max_output_tokens"],
        "wire_stream": bodies[0]["stream"], "model_requests": model.request_count(),
        "decision": Value::Null, "error_code": "exo_turn_failed",
        "fetch_attempts": 1,
        "evidence": "provider-truncation-is-not-a-decision"}),
    );
    Ok(())
}
