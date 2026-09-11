// SPDX-License-Identifier: MIT

use super::*;
use recorded_run_json::parse;
use recorded_run_projection::project;
use serde_json::json;

fn accounting() -> Value {
    json!({"schema":"sts2.provider-accounting-v1","execution_status":"completed","decision_status":"valid",
        "usage_status":"reported","usage":{"input_tokens":10},"provider_request_identity_status":"reported",
        "provider_request_id_kind":"codex_thread_id","provider_request_id":"private"})
}
#[test]
fn accounting_does_not_invent_statuses_or_usage() -> Result<(), String> {
    let baseline = accounting();
    for (field, invalid) in [
        ("schema", "foreign"),
        ("execution_status", "failed"),
        ("decision_status", "rejected"),
        ("usage_status", "other"),
        ("provider_request_id_kind", "other"),
        ("provider_request_identity_status", "other"),
    ] {
        let mut row = baseline.clone();
        row[field] = json!(invalid);
        let projected = project(
            "provider-accounting",
            Some(&serde_json::to_vec(&row).map_err(|e| e.to_string())?),
            |v, n| accounting_record(v, n).map(Some),
        )?;
        assert!(projected.records.is_empty());
        assert_eq!(projected.report["unsupported_rows"], 1);
    }
    let record = accounting_record(&baseline, 0)?;
    assert!(
        record
            .pointer("/payload/value/usage/reasoning_output_tokens")
            .is_none()
    );
    assert!(record.pointer("/payload/value/input_written").is_none());
    for status in ["unknown", "not_applicable"] {
        let mut row = baseline.clone();
        row["usage_status"] = json!(status);
        let record = accounting_record(&row, 0)?;
        assert_eq!(
            record["payload"]["value"]["usage"]["input_tokens"]["value_status"],
            status
        );
        assert!(record["payload"]["value"]["usage"]["input_tokens"]["value"].is_null());
    }
    Ok(())
}
#[test]
fn output_bounds_and_scoped_revisions() -> Result<(), String> {
    assert!(ndjson(&[json!("x".repeat(65_536))]).is_err());
    assert!(
        recorded_run_support::accounting_counts(&json!({"turn_count":1,"completed_turn_count":2}))
            .is_err()
    );
    assert!(process_result(&json!({"exit_code":0,"session_id":"private path"})).is_err());
    let heads = json!({"metadata":{"heads":{
        "protocol":"ed8626c2cf30089b4bdf214a2fdcb09b3eca3d29",
        "harness":"682c2b5ba38010e16d43b04c43d40184bda70106",
        "game-mod":"70c56c3f5bb179b32b3e919814c43d977af2b725"
    },"native_package":{"source_head":"different"},"live_retry":{"merged_commit":"different"}}});
    assert_eq!(
        source_versions(&heads),
        json!({
            "protocol":"ed8626c2cf30089b4bdf214a2fdcb09b3eca3d29",
            "runtime":"682c2b5ba38010e16d43b04c43d40184bda70106",
            "game_mod":"70c56c3f5bb179b32b3e919814c43d977af2b725"
        })
    );
    Ok(())
}

#[test]
fn followup_optional_decision_and_invalid_accounting_classes() -> Result<(), String> {
    let row = json!({"event":"model_decision","model_execution_id":1,"action_id":"a"});
    let p = project(
        "trajectory",
        Some(&serde_json::to_vec(&row).map_err(|e| e.to_string())?),
        trajectory_record,
    )?;
    assert_eq!(p.records.len(), 1);
    assert!(p.records[0].pointer("/payload/value/observation").is_none());
    assert_eq!(
        p.records[0]["identities"]["action"]["value"],
        privacy_digest("action", "a")
    );
    for mutation in [
        json!({"usage":{"input_tokens":-1}}),
        json!({"usage":"wrong-container"}),
        json!({"harness_model_execution_id":"bad+identity"}),
    ] {
        let mut row = accounting();
        for (key, value) in mutation.as_object().ok_or("fixture")? {
            row[key] = value.clone();
        }
        let p = project(
            "provider-accounting",
            Some(&serde_json::to_vec(&row).map_err(|e| e.to_string())?),
            |v, n| accounting_record(v, n).map(Some),
        )?;
        assert_eq!(p.report["rejected_rows"], 1);
        assert_eq!(p.report["unsupported_rows"], 0);
        assert_eq!(
            p.report["dispositions"][0]["reason"],
            "invalid_source_record"
        );
    }
    assert!(process_result(&json!({"session_id":"session+1","exit_code":0})).is_err());
    let mut row = accounting();
    row["provider"] = json!("private-config-label");
    row["model"] = json!("private-model-label");
    let record = accounting_record(&row, 0)?;
    assert!(record.pointer("/payload/value/provider").is_none());
    assert!(record.pointer("/payload/value/model").is_none());
    Ok(())
}

#[test]
fn omissions_member_limit_and_compact_mcp_ranges() -> Result<(), String> {
    assert!(
        matches!(project("mcp", Some(&b"\n".repeat(25_001)), |_,_| Ok(None)),
        Err(e) if e == "source_record_count")
    );
    let p = project("mcp", Some(&b"{}\n".repeat(25_000)), |_, _| Ok(None))?;
    assert_eq!(p.report["input_records"], 25_000);
    assert_eq!(
        p.report["dispositions"].as_array().ok_or("ranges")?.len(),
        1
    );
    assert_eq!(p.report["dispositions"][0]["last"], 24_999);
    let output = tests::root("oversized-omissions.zip");
    let entry = Entry::new(
        "reports/omissions.json",
        "application/json",
        vec![b' '; 1_048_577],
    );
    assert!(write_zip(&output, &[entry]).is_err());
    assert!(!output.exists());
    Ok(())
}

#[test]
fn physical_tail_and_absence_are_reconciled() -> Result<(), String> {
    let row = br#"{"event":"episode_failed","error_code":"other"}"#;
    let bytes = [row.as_slice(), b"\n{\"event\":"].concat();
    let projected = project("trajectory", Some(&bytes), trajectory_record)?;
    assert_eq!(projected.records.len(), 1);
    assert_eq!(projected.report["input_records"], 2);
    assert_eq!(projected.report["rejected_rows"], 1);
    assert_eq!(projected.report["dispositions"][0]["first"], 1);
    assert_eq!(projected.report["state"], "interrupted");
    let blank = [row.as_slice(), b"\n\n", row.as_slice()].concat();
    assert!(project("trajectory", Some(&blank), trajectory_record).is_err());
    let absent = project("provider-accounting", None, |_, _| Ok(None))?;
    assert_eq!(absent.report["state"], "absent");
    assert!(absent.report["input_records"].is_null());
    Ok(())
}
#[test]
fn omissions_and_action_correlation_survive() -> Result<(), String> {
    let row = json!({"event":"action_receipt","status":"Unknown","effect":null,"action_id":"play:1","operation_id":"op"});
    let encoded = serde_json::to_vec(&row).map_err(|e| e.to_string())?;
    let p = project("trajectory", Some(&encoded), trajectory_record)?;
    assert_eq!(
        p.records[0]["identities"]["action"]["value"],
        privacy_digest("action", "play:1")
    );
    assert_eq!(
        p.report["field_omissions"][0]["rule"],
        "identity_digest_transformation"
    );
    let p = project(
        "provider-accounting",
        Some(&serde_json::to_vec(&accounting()).map_err(|e| e.to_string())?),
        |v, n| accounting_record(v, n).map(Some),
    )?;
    assert_eq!(
        p.report["field_omissions"][0]["rule"],
        "provider_request_id_disallowed"
    );
    let mut invalid = row;
    invalid["status"] = json!("Settled");
    assert!(trajectory_record(&invalid, 0)?.is_none());
    Ok(())
}
#[test]
fn jcs_vectors_and_duplicate_keys() -> Result<(), String> {
    let value = parse(br#"{"x":1e30,"tiny":1e-7,"zero":-0.0,"big":1e20}"#)?;
    assert_eq!(
        String::from_utf8(canonical(&value)?).map_err(|e| e.to_string())?,
        r#"{"big":100000000000000000000,"tiny":1e-7,"x":1e+30,"zero":0}"#
    );
    let value = json!({"\u{e000}":1,"\u{10000}":2});
    assert_eq!(
        String::from_utf8(canonical(&value)?).map_err(|e| e.to_string())?,
        "{\"\u{10000}\":2,\"\u{e000}\":1}"
    );
    assert!(parse(br#"{"x":1,"\u0078":2}"#).is_err());
    assert!(parse(br#"{"x":"\ud800"}"#).is_err());
    assert!(parse(format!("{}0{}", "[".repeat(40), "]".repeat(40)).as_bytes()).is_err());
    assert_eq!(
        parse(b"9007199254740993")?.as_u64(),
        Some(9_007_199_254_740_993)
    );
    Ok(())
}
#[test]
fn seed_requires_context_provenance_and_generation_advance() -> Result<(), String> {
    let golden: Value = serde_json::from_str(include_str!(
        "../../../protocol-artifact/seeded-run-v1/golden/start-settled.json"
    ))
    .map_err(|e| e.to_string())?;
    let receipt = json!({"settled":golden});
    assert!(recorded_run_seed::valid(&receipt));
    for pointer in [
        "/settled/schema_digest",
        "/settled/observation/generation",
        "/settled/effect_witness/generation",
        "/settled/observation/selected_context_digest",
        "/settled/provenance/generator",
    ] {
        let mut mutated = receipt.clone();
        *mutated
            .pointer_mut(pointer)
            .ok_or_else(|| String::from("test_pointer"))? = Value::Null;
        assert!(!recorded_run_seed::valid(&mutated));
    }
    Ok(())
}
