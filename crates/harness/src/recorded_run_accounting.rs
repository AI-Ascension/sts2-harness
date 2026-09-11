// SPDX-License-Identifier: MIT

use super::recorded_run_support::accounting_counts;
use super::{SEED, envelope, required_str, unknown_evidence};
use serde_json::{Map, Value, json};

pub(super) fn accounting_record(row: &Value, ordinal: usize) -> Result<Value, String> {
    if row["schema"] != "sts2.provider-accounting-v1" {
        return Err(String::from("unsupported_accounting_schema"));
    }
    let execution = admitted(row, "execution_status", &["completed", "unknown"])?;
    let decision = admitted(row, "decision_status", &["valid", "unknown"])?;
    let usage_status = admitted(
        row,
        "usage_status",
        &["reported", "unknown", "not_applicable"],
    )?;
    let mut tokens = Map::new();
    if row
        .get("usage")
        .is_some_and(|v| !v.is_object() && !v.is_null())
    {
        return Err(String::from("invalid_usage_container"));
    }
    for key in [
        "input_tokens",
        "output_tokens",
        "cached_input_tokens",
        "cache_write_input_tokens",
        "reasoning_output_tokens",
    ] {
        let Some(value) = row.get("usage").and_then(|v| v.get(key)) else {
            continue;
        };
        let (status, value) = if usage_status == "reported" && !value.is_null() {
            let number = value
                .as_u64()
                .ok_or_else(|| String::from("invalid_usage_count"))?;
            ("reported", json!(number.to_string()))
        } else {
            (
                if usage_status == "not_applicable" {
                    "not_applicable"
                } else {
                    "unknown"
                },
                Value::Null,
            )
        };
        tokens.insert(
            key.to_owned(),
            json!({"scope":"model_execution","unit":"tokens","value_status":status,"value":value}),
        );
    }
    let mut value = json!({"profile":SEED,"variant":"accounting","source_schema":"sts2.provider-accounting-v1",
        "provider_execution_status":execution,"decision_status":decision,"counts":accounting_counts(row)?,"usage":tokens});
    for (source, target, vocabulary) in [
        (
            "provider_request_id_kind",
            "provider_request_identity_kind",
            &["codex_thread_id", "unknown"][..],
        ),
        (
            "provider_request_identity_status",
            "provider_request_identity_status",
            &["reported", "unknown"][..],
        ),
    ] {
        if row.get(source).is_some() {
            value[target] = json!(admitted(row, source, vocabulary)?);
        }
    }
    // Labels are optional. No source-backed public-label allowlist is pinned yet;
    // lexical validity alone cannot establish that a configured label is public.
    for key in ["request_sha256", "decision_sha256"] {
        if let Some(field) = row.get(key) {
            let text = field
                .as_str()
                .ok_or_else(|| String::from("invalid_accounting_field"))?;
            let valid = text.len() == 64
                && text
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
            if !valid {
                return Err(String::from("invalid_accounting_token"));
            }
            value[key] = json!(text);
        }
    }
    if let Some(written) = row.get("input_written") {
        value["input_written"] = json!(
            written
                .as_bool()
                .ok_or_else(|| String::from("input_written_type"))?
        );
    }
    let mut ids = json!({});
    if let Some(execution) = row.get("harness_model_execution_id") {
        let text = execution
            .as_str()
            .ok_or_else(|| String::from("execution_identity_type"))?;
        if !super::recorded_run_support::identity_token(text, 256) {
            return Err(String::from("execution_identity"));
        }
        ids["model_execution"] =
            json!({"namespace":"seed-readiness.accounting.model-execution","value":text});
    }
    Ok(envelope(
        "accounting",
        json!({"stream":"provider-accounting","record_ordinal":ordinal}),
        ids,
        unknown_evidence(),
        value,
    ))
}
fn admitted<'a>(row: &'a Value, field: &str, allowed: &[&str]) -> Result<&'a str, String> {
    let value = required_str(row, field)?;
    if !allowed.contains(&value) {
        return Err(String::from("unsupported_source_status"));
    }
    Ok(value)
}
