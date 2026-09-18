// SPDX-License-Identifier: MIT

//! Real-process proof for `sts2-harness#141` AC2: the shipped `sts2-exo-bridge` advertises exactly
//! the variants it implements, and every non-inferencing probe makes **zero** model requests.
//!
//! This is deliberately a separate file from `process_oracle.rs`: that oracle's bytes are pinned by
//! `docs/evidence/exo-executor-process-oracle-20260915.json`, so extending it would invalidate the
//! recorded digest instead of adding evidence. Evidence class here is **real-process with a
//! synthetic loopback model and no game** — it is not native-game and not real-provider evidence.

mod support;

use serde_json::{Value, json};
use std::path::PathBuf;
use support::{Model, Result, digest, invoke, response};

#[test]
#[ignore = "requires built bridge, pinned Exo source/dependencies and Node; see README"]
fn advertised_variants_and_zero_model_probes() -> Result {
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
    let config = root.join("target/exo-advertised-config.json");
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

    // The model is armed with a decision that *would* succeed, so a probe that wrongly inferred
    // would visibly contact the model rather than failing for an unrelated reason.
    model.set(
        200,
        response(
            r#"{"decision":"reobserve","rationale":"synthetic"}"#,
            "message",
        ),
    )?;

    let described = invoke(&binary, &config, b"", "--describe", true)?;
    assert!(described.status.success());
    let capability: Value = serde_json::from_slice(&described.stdout)?;
    assert_eq!(capability["schema"], json!("sts2.exo-one-shot-capability-v1"));
    assert_eq!(capability["profiles"], json!(["standard"]));
    assert_eq!(capability["context_modes"], json!(["fresh"]));
    assert_eq!(
        capability["decisions"],
        json!(["action", "plan", "wait", "reobserve"])
    );
    // Unsupported variants are named, not merely absent.
    assert_eq!(capability["profile_support"]["map"], json!("unsupported"));
    // `management` is enforced by the shared classifier, so it must be discoverable here too.
    // It was previously omitted, leaving a caller unable to pre-check it.
    assert_eq!(
        capability["profile_support"]["management"],
        json!("unsupported")
    );
    assert_eq!(capability["profile_support"]["expert"], json!("unsupported"));
    assert_eq!(
        capability["decision_support"]["recovery"],
        json!("unsupported")
    );
    assert_eq!(
        capability["unsupported_profile_code"],
        json!("exo_bridge_unsupported_profile")
    );
    assert_eq!(
        capability["unsupported_recovery_code"],
        json!("exo_bridge_unsupported_recovery")
    );
    assert_eq!(capability["full_runtime_admission"], json!(false));
    assert_eq!(capability["model_calls"], json!(0));
    // The advertised endpoint is the synthetic loopback, never a real provider.
    assert!(
        capability["endpoint"]
            .as_str()
            .is_some_and(|endpoint| endpoint.starts_with("http://127.0.0.1:"))
    );
    assert_eq!(model.request_count(), 0);
    assert!(model.requests.lock().map_err(|_| "poisoned")?.is_empty());

    // Repeating a non-inferencing probe is idempotent and still makes no request.
    let repeated = invoke(&binary, &config, b"", "--describe", true)?;
    assert!(repeated.status.success());
    assert_eq!(repeated.stdout, described.stdout);
    assert_eq!(model.request_count(), 0);

    // A map-profile request is refused by the shared profile guard before any inference, and the
    // refusal is the same typed fail-closed code the advertisement publishes.
    let golden: Value = serde_json::from_slice(&std::fs::read(
        root.join("protocol-artifact/exo-bridge-v1/golden/request.json"),
    )?)?;
    // `ordinary_map` transforms a complete request/turn envelope, not a bare request.
    let base_envelope = json!({
        "wire_version": "sts2.exo-bridge-wire-v1",
        "request_id": "host-request-map",
        "turn_id": "host-turn-map",
        "request": golden
    });
    let map_envelope = support::ordinary_map(&base_envelope)?;
    let map = invoke(
        &binary,
        &config,
        &serde_json::to_vec(&map_envelope)?,
        "--synthetic",
        true,
    )?;
    assert!(!map.status.success());
    assert!(map.stdout.is_empty());
    assert_eq!(map.stderr, b"exo_bridge_unsupported_profile\n");
    assert_eq!(model.request_count(), 0);
    assert!(model.requests.lock().map_err(|_| "poisoned")?.is_empty());

    // A management-profile request is refused by the same shared guard with the same published
    // code. Without this probe the axis was enforced but neither advertised nor exercised here.
    let mut management_envelope = base_envelope.clone();
    management_envelope["request"]["management_profile"] = json!("management-enabled");
    // The contract requires a non-null context alongside an enabled profile, so the request is
    // schema-valid and the shared profile guard is the only thing that can refuse it.
    management_envelope["request"]["management_context"] = json!({});
    let management = invoke(
        &binary,
        &config,
        &serde_json::to_vec(&management_envelope)?,
        "--synthetic",
        true,
    )?;
    assert!(!management.status.success());
    assert!(management.stdout.is_empty());
    assert_eq!(management.stderr, b"exo_bridge_unsupported_profile\n");
    assert_eq!(model.request_count(), 0);
    assert!(model.requests.lock().map_err(|_| "poisoned")?.is_empty());

    // A config-integrity rejection is also pre-inference: the packaged executor digest no longer
    // matches, so the bridge refuses before it can spawn the executor or contact the model.
    let tampered_config = root.join("target/exo-advertised-tampered-config.json");
    let mut tampered: Value = serde_json::from_slice(&std::fs::read(&config)?)?;
    tampered["executor_sha256"] = json!("0".repeat(64));
    std::fs::write(&tampered_config, serde_json::to_vec(&tampered)?)?;
    let rejected = invoke(&binary, &tampered_config, b"", "--describe", true)?;
    assert!(!rejected.status.success());
    assert!(rejected.stdout.is_empty());
    assert_eq!(model.request_count(), 0);
    std::fs::remove_file(tampered_config)?;

    let report = json!({
        "schema": "sts2.exo-advertised-variant-evidence-v1",
        "evidence": "real-process-synthetic-model-no-game",
        "bridge_sha256": digest(&binary)?,
        "executor_sha256": digest(&executor)?,
        "extension_sha256": digest(&extension)?,
        "harness_revision": String::from_utf8(std::process::Command::new("git")
            .arg("-C").arg(&root).args(["rev-parse", "HEAD"]).output()?.stdout)?.trim(),
        "oracle_sha256": digest(&PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/advertised_variant_oracle.rs"))?,
        "probes": ["describe", "describe_repeated", "map_refused_pre_inference",
            "management_refused_pre_inference", "tampered_config_rejected"],
        "model_requests": 0,
        "advertised_profiles": ["standard"],
        "advertised_decisions": ["action", "plan", "wait", "reobserve"],
        "full_runtime_admission": false
    });
    std::fs::write(
        root.join("target/exo-advertised-report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    std::fs::remove_file(config)?;
    Ok(())
}
