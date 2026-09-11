// SPDX-License-Identifier: MIT

use serde_json::{Map, Value, json};

use super::recorded_run_encoding::canonical;
use super::{COMMON, SEED};
use crate::sha256_hex;

pub(super) fn accounting_counts(row: &Value) -> Result<Value, String> {
    let mut counts = Map::new();
    for key in [
        "stdout_bytes",
        "stderr_bytes",
        "event_count",
        "turn_count",
        "completed_turn_count",
    ] {
        if let Some(value) = row.get(key) {
            let value = value
                .as_u64()
                .ok_or_else(|| format!("accounting {key} invalid"))?;
            counts.insert(key.to_owned(), Value::String(value.to_string()));
        }
    }
    if let (Some(turns), Some(completed)) = (
        row.get("turn_count").and_then(Value::as_u64),
        row.get("completed_turn_count").and_then(Value::as_u64),
    ) && completed > turns
    {
        return Err(String::from("invalid_completed_turn_count"));
    }
    Ok(Value::Object(counts))
}

pub(super) fn process_result(row: &Value) -> Result<Value, String> {
    let exit = row
        .get("exit_code")
        .and_then(Value::as_i64)
        .ok_or_else(|| String::from("result exit_code invalid"))?;
    if i32::try_from(exit).is_err() || !identity_token(required_str(row, "session_id")?, 256) {
        return Err(String::from("invalid_process_result"));
    }
    let evidence = json!({"process_exit":if exit == 0 {"completed"} else {"failed"},"request":"unknown","action":"unknown","outcome":"unknown","gameplay":"unknown"});
    Ok(envelope(
        "diagnostic",
        json!({"stream":"result","record_ordinal":0}),
        json!({"session":{"namespace":"sts2.recorded-run.result-session","value":required_str(row,"session_id")?}}),
        evidence,
        json!({"profile":SEED,"variant":"process_result","exit_code":exit}),
    ))
}

pub(super) fn envelope(
    _kind: &str,
    mut source: Value,
    identities: Value,
    evidence: Value,
    mut payload: Value,
) -> Value {
    source["subrecord_ordinal"] = json!(0);
    let kind = if let Some(object) = payload.as_object_mut() {
        let kind = object.remove("variant").unwrap_or(Value::Null);
        object.remove("profile");
        kind
    } else {
        Value::Null
    };
    json!({"profile":COMMON,"source":source,"identities":identities,"evidence":evidence,"payload":{"profile":SEED,"kind":kind,"value":payload}})
}
pub(super) fn unknown_evidence() -> Value {
    json!({"process_exit":"unknown","request":"unknown","action":"unknown","outcome":"unknown","gameplay":"unknown"})
}
pub(super) fn identities(source: &Value, fields: &[&str]) -> Result<Value, String> {
    let mut ids = Map::new();
    if let Some(action) = source.get("action_id") {
        let action = action
            .as_str()
            .ok_or_else(|| String::from("invalid_action_id"))?;
        ids.insert(
            "action".to_owned(),
            json!({"namespace":"ai-ascension.action.sha256",
            "value":privacy_digest("action",action)}),
        );
    }
    for field in fields {
        if let Some(value) = source.get(*field).and_then(Value::as_str) {
            if !identity_token(value, 256) {
                return Err(String::from("invalid_identity"));
            }
            ids.insert(
                field.trim_end_matches("_id").to_owned(),
                json!({"namespace":format!("sts2.seed-readiness.{field}"),"value":value}),
            );
        }
    }
    Ok(Value::Object(ids))
}
pub(super) fn token(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:+-".contains(&b))
}
pub(super) fn identity_token(value: &str, maximum: usize) -> bool {
    token(value, maximum) && !value.contains('+')
}
pub(super) fn required_str<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing or invalid string: {key}"))
}
pub(super) fn privacy_digest(domain: &str, value: &str) -> String {
    sha256_hex(
        [
            b"ai-ascension.recorded-run.v1/".as_slice(),
            domain.as_bytes(),
            &[0],
            value.as_bytes(),
        ]
        .concat(),
    )
}
pub(super) fn privacy_value_digest(domain: &str, value: &Value) -> Result<String, String> {
    Ok(privacy_digest(
        domain,
        std::str::from_utf8(&canonical(value)?)
            .map_err(|_| String::from("canonical json was not UTF-8"))?,
    ))
}
