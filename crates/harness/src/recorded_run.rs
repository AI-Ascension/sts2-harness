// SPDX-License-Identifier: MIT

//! Bounded, privacy-preserving export of the legacy Train recording directory.
//! The record envelope is deliberately limited to the reviewed candidate fields.

use std::path::Path;

use serde_json::{Value, json};

use crate::sha256_hex;

#[path = "recorded_run_accounting.rs"]
mod recorded_run_accounting;
#[path = "recorded_run_encoding.rs"]
mod recorded_run_encoding;
#[path = "recorded_run_json.rs"]
mod recorded_run_json;
#[path = "recorded_run_observation.rs"]
mod recorded_run_observation;
#[path = "recorded_run_projection.rs"]
mod recorded_run_projection;
#[path = "recorded_run_seed.rs"]
mod recorded_run_seed;
#[path = "recorded_run_snapshot.rs"]
mod recorded_run_snapshot;
#[path = "recorded_run_support.rs"]
mod recorded_run_support;
#[path = "recorded_run_trajectory.rs"]
mod recorded_run_trajectory;
#[cfg(test)]
#[path = "recorded_run_review_tests.rs"]
mod review_tests;
#[cfg(test)]
#[path = "recorded_run_snapshot_tests.rs"]
mod snapshot_tests;
#[cfg(test)]
#[path = "recorded_run_tests.rs"]
mod tests;

use recorded_run_accounting::accounting_record;
use recorded_run_encoding::{Entry, canonical, ndjson, write_zip};
use recorded_run_observation::observation_summary;
use recorded_run_projection::project;
use recorded_run_snapshot::Snapshot;
use recorded_run_support::{
    envelope, identities, privacy_digest, privacy_value_digest, process_result, required_str,
    unknown_evidence,
};
use recorded_run_trajectory::trajectory_record;

const FORMAT: &str = "seed-readiness-controller-release-v2";
const MAX_FILE_BYTES: usize = 16 * 1024 * 1024;
const MAX_LINE_BYTES: usize = 1024 * 1024;
const COMMON: &str = "ai-ascension.recorded-run.common.v1";
const SEED: &str = "ai-ascension.sts2.seed-readiness.v1";
const CANDIDATE_SCHEMA_SHA256: &str =
    "a6c32127290f4d5e670d8863f97a74a7b8e3e411e735d81394b51fe1578b4eb6";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportReport {
    pub bundle_semantic_digest: String,
    pub emitted_events: usize,
    pub emitted_accounting: usize,
}

pub fn export_directory(input: &Path, output: &Path) -> Result<ExportReport, String> {
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if parent
        .canonicalize()
        .map_err(|_| String::from("output_parent"))?
        .starts_with(
            input
                .canonicalize()
                .map_err(|_| String::from("source_root"))?,
        )
    {
        return Err(String::from("output_inside_source"));
    }
    let snapshot = Snapshot::read(input)?;
    let source_manifest = recorded_run_json::parse(snapshot.required("manifest.json")?)?;
    if source_manifest.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err(String::from("unsupported_source_format"));
    }
    let trajectory = project(
        "trajectory",
        Some(snapshot.required("trajectory.jsonl")?),
        trajectory_record,
    )?;
    let decisions = project(
        "decisions",
        Some(snapshot.required("decisions.jsonl")?),
        |v, n| decision_record(v, n).map(Some),
    )?;
    let mcp = project("mcp", Some(snapshot.required("mcp.jsonl")?), |_, _| {
        Ok(None)
    })?;
    let accounting = project(
        "provider-accounting",
        snapshot
            .files
            .get("provider-accounting.jsonl")
            .map(Vec::as_slice),
        |v, n| accounting_record(v, n).map(Some),
    )?;
    let result = recorded_run_json::parse(snapshot.required("result.json")?)?;
    let mut events = decisions.records;
    events.push(process_result(&result)?);
    events.extend(trajectory.records);
    let accounting_records = accounting.records;
    let manifest_report = json!({"stream":"manifest","state":"omitted","input_records":1,
        "emitted_rows":0,"filtered_rows":1,"unsupported_rows":0,"rejected_rows":0,
        "output_records":0,"dispositions":[{"first":0,"last":0,"disposition":"filtered",
        "reason":"private_source_metadata"}],"field_omissions":[]});
    let mut result_report = json!({"stream":"result","state":"present","input_records":1,
        "emitted_rows":1,"filtered_rows":0,"unsupported_rows":0,"rejected_rows":0,
        "output_records":1,"dispositions":[],"field_omissions":[{"rule":"private_source_metadata","affected_rows":1}]});
    if result.as_object().is_some_and(|v| {
        v.keys()
            .all(|k| matches!(k.as_str(), "session_id" | "exit_code"))
    }) {
        result_report["field_omissions"] = json!([]);
    }
    let omissions = json!({"profile":COMMON,"fixture_provenance":"sanitized_source_derived",
        "streams":[decisions.report,manifest_report,mcp.report,accounting.report,result_report,trajectory.report]});
    let event_bytes = ndjson(&events)?;
    let accounting_bytes = ndjson(&accounting_records)?;
    let omissions_bytes = canonical(&omissions)?;
    let mut entries = vec![
        Entry::new(
            "records/accounting.ndjson",
            "application/x-ndjson",
            accounting_bytes,
        ),
        Entry::new("records/events.ndjson", "application/x-ndjson", event_bytes),
        Entry::new(
            "reports/omissions.json",
            "application/json",
            omissions_bytes,
        ),
    ];
    if !snapshot.files.contains_key("provider-accounting.jsonl") {
        entries.remove(0);
    }
    let recording_identity = json!({"namespace":"seed-readiness.recording.sha256","value":privacy_digest("run", input.file_name().and_then(|value| value.to_str()).unwrap_or("recording"))});
    let mut manifest = json!({
        "format":"ai-ascension.recorded-run-bundle",
        "format_version":"1.0.0-candidate.3",
        "contract_schema_sha256":CANDIDATE_SCHEMA_SHA256,
        "required_profiles":[COMMON,SEED],
        "optional_profiles":[],
        "bundle_id":privacy_value_digest("bundle-id",&recording_identity)?,
        "recording":{"identity":recording_identity,"identities":{}},
        "producer":{"name":"seed-readiness-controller","version":"unknown","source_format":FORMAT},
        "adapter":{"name":"sts2-recorded-run-export","version":env!("CARGO_PKG_VERSION"),"source_revision":adapter_revision()},
        "versions":source_versions(&source_manifest),
        "capabilities":{"inspection":true,"execution_replay":false,"live_control":false,"raw_mcp":false},
        "completeness":{"status":"partial","source_snapshot":"unverified"},
        "evidence":{"process_exit":if result.get("exit_code").and_then(Value::as_i64)==Some(0) {"completed"} else {"failed"},"request":"unknown","action":"unknown","outcome":"unknown","gameplay":if events.iter().any(|row|row.pointer("/payload/value/code").and_then(Value::as_str)==Some("episode_failed")) {"episode_failed"} else {"unknown"}},
        "entries":entries.iter().map(Entry::manifest).collect::<Vec<_>>(),
        "integrity":{"digest_algorithm":"sha-256"}
    });
    let semantic = sha256_hex(canonical(&manifest)?);
    manifest["integrity"]["bundle_semantic_digest"] = Value::String(semantic.clone());
    entries.push(Entry::new(
        "manifest.json",
        "application/json",
        canonical(&manifest)?,
    ));
    if events.len() + accounting_records.len() > 25_000 {
        return Err(String::from("record_count"));
    }
    snapshot.verify(input)?;
    write_zip(output, &entries)?;
    Ok(ExportReport {
        bundle_semantic_digest: semantic,
        emitted_events: events.len(),
        emitted_accounting: accounting_records.len(),
    })
}

/// Finalization command helper for the external release controller.
///
/// It must be invoked only after the controller has closed all JSONL writers and finalized
/// result.json. The harness runtime itself deliberately does not call this helper.
pub fn finalize_from_environment() -> Result<(), String> {
    let source = std::env::var_os("STS2_RECORDED_RUN_SOURCE_DIR");
    let output = std::env::var_os("STS2_RECORDED_RUN_BUNDLE_PATH");
    match (source, output) {
        (None, None) => Ok(()),
        (Some(source), Some(output)) => {
            finalize_after_controller(Path::new(&source), Path::new(&output)).map(|_| ())
        }
        _ => Err(String::from(
            "recorded-run finalization requires both source and output paths",
        )),
    }
}

/// Export a controller-owned run only after its controller has completed shutdown.
///
/// The release controller writes `result.json` after waiting for the harness, watcher, and
/// gateway. Its post-wait wrapper invokes this boundary; the required-result check prevents an
/// accidental harness-exit invocation from creating a partial bundle.
pub fn finalize_after_controller(input: &Path, output: &Path) -> Result<ExportReport, String> {
    export_directory(input, output)
}

fn decision_record(row: &Value, ordinal: usize) -> Result<Value, String> {
    let time = row
        .get("time_ns")
        .and_then(Value::as_u64)
        .ok_or_else(|| String::from("decision time_ns invalid"))?;
    let decision = row
        .get("decision")
        .and_then(Value::as_object)
        .ok_or_else(|| String::from("decision object missing"))?;
    let ids = decision
        .get("action_ids")
        .and_then(Value::as_array)
        .ok_or_else(|| String::from("decision action_ids missing"))?
        .iter()
        .map(|v| {
            v.as_str()
                .map(|s| privacy_digest("action", s))
                .ok_or_else(|| String::from("decision action id invalid"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut record = envelope(
        "decision",
        json!({"stream":"decisions","record_ordinal":ordinal}),
        json!({}),
        unknown_evidence(),
        json!({"profile":SEED,"variant":"decision_summary","action_id_digests":ids}),
    );
    record["time"] = json!({"unix_ns":time.to_string()});
    Ok(record)
}

fn source_versions(manifest: &Value) -> Value {
    let mut versions = serde_json::Map::new();
    for (source, target) in [
        ("protocol", "protocol"),
        ("harness", "runtime"),
        ("game-mod", "game_mod"),
    ] {
        if let Some(revision) = manifest
            .pointer(&format!("/metadata/heads/{source}"))
            .and_then(Value::as_str)
            .filter(|s| s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            versions.insert(target.to_owned(), json!(revision));
        }
    }
    Value::Object(versions)
}
fn adapter_revision() -> String {
    // Content identity of the adapter source is independent of the recorded producer.
    sha256_hex(concat!(
        include_str!("recorded_run.rs"),
        include_str!("recorded_run_accounting.rs"),
        include_str!("recorded_run_trajectory.rs"),
        include_str!("recorded_run_support.rs"),
        include_str!("recorded_run_observation.rs"),
        include_str!("recorded_run_projection.rs"),
        include_str!("recorded_run_snapshot.rs"),
        include_str!("recorded_run_json.rs"),
        include_str!("recorded_run_seed.rs"),
        include_str!("recorded_run_encoding.rs"),
        include_str!("bin/sts2-recorded-run-export.rs"),
        include_str!("../Cargo.toml"),
        include_str!("../../../Cargo.toml"),
        include_str!("../../../rust-toolchain.toml"),
        include_str!("../../../Cargo.lock")
    ))
}
