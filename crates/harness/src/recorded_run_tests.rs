// SPDX-License-Identifier: MIT

use std::fs;
use std::path::{Path, PathBuf};

use super::*;

pub(super) fn root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("sts2-recorded-run-{name}-{}", std::process::id()))
}

pub(super) fn write_fixture(root: &Path, malformed: bool) -> Result<(), String> {
    let _ = fs::remove_dir_all(root);
    fs::create_dir_all(root).map_err(|error| error.to_string())?;
    fs::write(
        root.join("manifest.json"),
        r#"{"format":"seed-readiness-controller-release-v2"}"#,
    )
    .map_err(|error| error.to_string())?;
    let suffix = if malformed { "not-json" } else { "" };
    fs::write(root.join("trajectory.jsonl"), format!(
            "{}{}",
            concat!(
                r#"{"event":"seeded_run_receipt","receipt":{"settled":{"instance_id":"instance","session_id":"session","lease_id":"lease","operation_id":"operation","correlation_id":"correlation","requested_seed":"SECRET_SEED","canonical_seed":"SECRET_SEED","generation":1,"protocol_version":"seeded-run-v1","schema_digest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","context_digest":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","status":"settled","observation":{"run_started":true,"host_ready":true,"generation":2,"canonical_seed":"SECRET_SEED"},"effect_witness":{"kind":"run_started","generation":2,"canonical_seed":"SECRET_SEED"}}}}"#,
                "\n",
                r#"{"event":"model_decision","model_execution_id":7,"reused_model_execution":false,"action_id":"SECRET_ACTION","observation":{"generation":1,"state_id":"SECRET_STATE","legal_actions":[]}}"#,
                "\n",
                r#"{"event":"action_receipt","operation_id":"operation","action_id":"SECRET_ACTION","status":"Unknown","effect":null,"observation":null}"#,
                "\n",
                r#"{"event":"operation_wait_completed","operation_id":"operation","action_id":"SECRET_ACTION","effect":"SECRET_EFFECT","observation":null}"#,
                "\n",
                r#"{"event":"episode_failed","error_code":"other"}"#,
                "\n"
            ),
            suffix
        )).map_err(|error| error.to_string())?;
    fs::write(root.join("decisions.jsonl"), r#"{"time_ns":9007199254740993,"provider_process":"SECRET_PROCESS","decision":{"action_ids":["SECRET_ACTION"],"rationale":"SECRET_RATIONALE"}}"#).map_err(|error| error.to_string())?;
    fs::write(root.join("mcp.jsonl"), r#"{"message":"SECRET_MCP"}"#)
        .map_err(|error| error.to_string())?;
    fs::write(root.join("provider-accounting.jsonl"), r#"{"harness_model_execution_id":"not-7","provider":"provider","model":"model","request_sha256":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","decision_sha256":"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd","usage":{"cache_write_input_tokens":1,"cached_input_tokens":2,"input_tokens":3,"output_tokens":4,"reasoning_output_tokens":5}}"#).map_err(|error| error.to_string())?;
    let path = root.join("provider-accounting.jsonl");
    let mut accounting: Value =
        serde_json::from_slice(&fs::read(&path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    for (key, value) in [
        ("schema", "sts2.provider-accounting-v1"),
        ("execution_status", "completed"),
        ("decision_status", "valid"),
        ("usage_status", "reported"),
        ("provider_request_id_kind", "codex_thread_id"),
        ("provider_request_identity_status", "reported"),
        ("provider_request_id", "SECRET_REQUEST"),
    ] {
        accounting[key] = json!(value);
    }
    fs::write(
        path,
        serde_json::to_vec(&accounting).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    fs::write(
        root.join("result.json"),
        r#"{"guest":"SECRET_GUEST","session_id":"session","exit_code":0}"#,
    )
    .map_err(|error| error.to_string())
}

#[test]
fn export_is_deterministic_and_redacts_source_text() -> Result<(), String> {
    let input = root("deterministic-input");
    write_fixture(&input, false)?;
    let first = root("first.zip");
    let second = root("second.zip");
    let first_report = export_directory(&input, &first)?;
    let second_report = export_directory(&input, &second)?;
    assert_eq!(first_report.emitted_accounting, 1);
    assert_eq!(
        first_report.bundle_semantic_digest,
        second_report.bundle_semantic_digest
    );
    assert_eq!(
        fs::read(&first).map_err(|error| error.to_string())?,
        fs::read(&second).map_err(|error| error.to_string())?
    );
    let bundle = fs::read(&first).map_err(|error| error.to_string())?;
    let text = String::from_utf8_lossy(&bundle);
    for forbidden in [
        "SECRET_SEED",
        "SECRET_ACTION",
        "SECRET_STATE",
        "SECRET_RATIONALE",
        "SECRET_PROCESS",
        "SECRET_MCP",
        "SECRET_GUEST",
    ] {
        assert!(!text.contains(forbidden));
    }
    assert!(text.contains("episode_failed"));
    assert!(text.contains("raw_mcp_disallowed"));
    let _ = fs::remove_dir_all(&input);
    let _ = fs::remove_file(&first);
    let _ = fs::remove_file(&second);
    Ok(())
}

#[test]
fn malformed_tail_preserves_prefix_in_partial_bundle() -> Result<(), String> {
    let input = root("malformed-input");
    let output = root("malformed.zip");
    write_fixture(&input, true)?;
    assert_eq!(export_directory(&input, &output)?.emitted_events, 7);
    assert!(output.exists());
    let _ = fs::remove_file(&output);
    let _ = fs::remove_dir_all(&input);
    Ok(())
}

#[test]
fn missing_final_result_refuses_premature_export() -> Result<(), String> {
    let input = root("missing-result-input");
    let output = root("missing-result.zip");
    write_fixture(&input, false)?;
    fs::remove_file(input.join("result.json")).map_err(|error| error.to_string())?;
    assert!(export_directory(&input, &output).is_err());
    assert!(!output.exists());
    let _ = fs::remove_dir_all(&input);
    Ok(())
}

#[test]
fn controller_finalizer_runs_only_after_result_is_finalized() -> Result<(), String> {
    let input = root("controller-finalizer-input");
    let premature = root("controller-premature.zip");
    let completed = root("controller-completed.zip");
    write_fixture(&input, false)?;
    let result = fs::read(input.join("result.json")).map_err(|error| error.to_string())?;
    fs::remove_file(input.join("result.json")).map_err(|error| error.to_string())?;

    // Fake controller before its post-wait result write: export must refuse to run.
    assert!(finalize_after_controller(&input, &premature).is_err());
    assert!(!premature.exists());

    // Fake controller's post-wait result write, then its harness-owned finalizer wrapper.
    fs::write(input.join("result.json"), result).map_err(|error| error.to_string())?;
    assert!(finalize_after_controller(&input, &completed).is_ok());
    assert!(completed.exists());
    let _ = fs::remove_dir_all(&input);
    let _ = fs::remove_file(&completed);
    Ok(())
}

#[test]
fn synthetic_bundle_passes_pinned_protocol_validator() -> Result<(), String> {
    let Some(validator) = std::env::var_os("STS2_RECORDED_RUN_VALIDATOR") else {
        return Ok(());
    };
    let input = root("protocol-validator-input");
    let output = root("protocol-validator.zip");
    write_fixture(&input, false)?;
    export_directory(&input, &output)?;
    let status = std::process::Command::new("node")
        .arg(validator)
        .arg(&output)
        .status()
        .map_err(|error| format!("cannot invoke pinned protocol validator: {error}"))?;
    assert!(
        status.success(),
        "pinned protocol validator rejected bundle"
    );
    let _ = fs::remove_dir_all(&input);
    let _ = fs::remove_file(&output);
    Ok(())
}
