// SPDX-License-Identifier: MIT

use super::{MAX_LINE_BYTES, recorded_run_json};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(super) struct Projection {
    pub records: Vec<Value>,
    pub report: Value,
}
pub(super) fn project(
    stream: &str,
    bytes: Option<&[u8]>,
    mapper: impl Fn(&Value, usize) -> Result<Option<Value>, String>,
) -> Result<Projection, String> {
    let mut report = json!({"stream":stream,"state":"absent","input_records":null,
        "emitted_rows":0,"filtered_rows":0,"unsupported_rows":0,"rejected_rows":0,
        "output_records":0,"dispositions":[],"field_omissions":[]});
    let Some(bytes) = bytes else {
        return Ok(Projection {
            records: vec![],
            report,
        });
    };
    let count = bytes.iter().filter(|b| **b == b'\n').count()
        + usize::from(!bytes.is_empty() && !bytes.ends_with(b"\n"));
    if count > 25_000 {
        return Err(String::from("source_record_count"));
    }
    let mut lines = bytes.split(|b| *b == b'\n').collect::<Vec<_>>();
    if lines.last() == Some(&&b""[..]) {
        lines.pop();
    }
    if lines.len() > 25_000 {
        return Err(String::from("source_record_count"));
    }
    report["state"] = json!("present");
    report["input_records"] = json!(lines.len());
    let mut records = Vec::new();
    let mut dispositions = Vec::new();
    let mut omissions = BTreeMap::<&str, usize>::new();
    let (mut filtered, mut unsupported, mut rejected, mut redacted) = (0, 0, 0, 0);
    for (ordinal, line) in lines.iter().enumerate() {
        if line.len() > MAX_LINE_BYTES {
            return Err(String::from("source_line_limit"));
        }
        let row = match recorded_run_json::parse(line) {
            Ok(row) => row,
            Err(_) if ordinal + 1 == lines.len() && !bytes.ends_with(b"\n") => {
                // Only a syntactically malformed tail is admitted; duplicate keys and limits
                // are invalid records even at EOF.
                if !serde_json::from_slice::<Value>(line).is_err_and(|e| {
                    (e.is_eof() || e.is_syntax()) && !e.to_string().contains("recursion limit")
                }) {
                    return Err(String::from("invalid_source_record"));
                }
                dispositions.push(json!({"first":ordinal,"last":ordinal,
                    "disposition":"rejected","reason":"partial_final_record"}));
                rejected += 1;
                report["state"] = json!("interrupted");
                continue;
            }
            Err(_) => return Err(String::from("invalid_source_record")),
        };
        if stream == "mcp" {
            redacted += row
                .get("redacted_paths")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            filtered += 1;
            dispositions.push(json!({"first":ordinal,"last":ordinal,"disposition":"filtered","reason":"raw_mcp_disallowed"}));
            continue;
        }
        match mapper(&row, ordinal) {
            Ok(Some(record)) => {
                records.push(record);
                for rule in field_rules(stream, &row) {
                    *omissions.entry(rule).or_default() += 1;
                }
            }
            result => {
                let is_unsupported = match result {
                    Ok(None) => true,
                    Err(error) => error.starts_with("unsupported_"),
                    Ok(Some(_)) => false,
                };
                let (class, reason) =
                    if is_unsupported && matches!(stream, "trajectory" | "provider-accounting") {
                        unsupported += 1;
                        ("unsupported", "unsupported_source_status")
                    } else {
                        rejected += 1;
                        ("rejected", "invalid_source_record")
                    };
                dispositions.push(
                    json!({"first":ordinal,"last":ordinal,"disposition":class,"reason":reason}),
                );
            }
        }
    }
    if stream == "mcp" {
        // Candidate3 requires all MCP rows filtered even when the private tail is malformed.
        // A truncated MCP stream cannot satisfy that invariant and must fail closed.
        if rejected != 0 {
            return Err(String::from("mcp_tail_requires_protocol_revision"));
        }
        report["state"] = json!("omitted");
        report["mcp_redacted_path_count"] = json!(redacted);
    }
    report["emitted_rows"] = json!(records.len());
    report["output_records"] = json!(records.len());
    report["filtered_rows"] = json!(filtered);
    report["unsupported_rows"] = json!(unsupported);
    report["rejected_rows"] = json!(rejected);
    let mut compact: Vec<Value> = Vec::new();
    for disposition in dispositions {
        if let Some(last) = compact.last_mut()
            && last["disposition"] == disposition["disposition"]
            && last["reason"] == disposition["reason"]
            && last["last"].as_u64().map(|n| n + 1) == disposition["first"].as_u64()
        {
            last["last"] = disposition["last"].clone();
        } else {
            compact.push(disposition);
        }
    }
    report["dispositions"] = json!(compact);
    report["field_omissions"] = json!(
        omissions
            .into_iter()
            .map(|(rule, affected_rows)| json!({"rule":rule,"affected_rows":affected_rows}))
            .collect::<Vec<_>>()
    );
    Ok(Projection { records, report })
}
fn field_rules(stream: &str, row: &Value) -> Vec<&'static str> {
    let mut rules = Vec::new();
    if row.get("action_id").is_some() || row.pointer("/decision/action_ids").is_some() {
        rules.push("identity_digest_transformation");
    }
    if row.pointer("/receipt/settled").is_some() {
        rules.push("raw_seed_disallowed");
    }
    if row.get("observation").is_some_and(|v| !v.is_null())
        || row
            .pointer("/receipt/settled/observation")
            .is_some_and(|v| !v.is_null())
    {
        rules.push("raw_observation_disallowed");
    }
    if row.pointer("/decision/rationale").is_some() {
        rules.push("raw_decision_text_disallowed");
    }
    if row.get("error_code").is_some() {
        rules.push("raw_error_disallowed");
    }
    if row.get("provider_request_id").is_some() {
        rules.push("provider_request_id_disallowed");
    }
    if stream == "result"
        || row.get("provider_process").is_some()
        || row.get("receipt").is_some()
        || (stream == "provider-accounting"
            && (row.get("provider").is_some() || row.get("model").is_some()))
    {
        rules.push("private_source_metadata");
    }
    rules
}
