// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::io::Write;
use std::process::{Command, Stdio};
use sts2_harness::context_memory::{MEMORY_QUERY_SCHEMA, MemoryScope, sha256_hex};

fn run(operation: &str, body: serde_json::Value) -> serde_json::Value {
    let mut child = Command::new(env!("CARGO_BIN_EXE_context-memory-cli"))
        .arg(operation)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn memory cli");
    let mut stdin = child.stdin.take().expect("stdin");
    write!(stdin, "{body}").expect("request");
    drop(stdin);
    let output = child.wait_with_output().expect("cli output");
    assert!(output.status.success());
    serde_json::from_slice(&output.stdout).expect("json response")
}

fn query(text: &str) -> serde_json::Value {
    serde_json::json!({
        "schema": MEMORY_QUERY_SCHEMA,
        "scope": MemoryScope::new("project-fixture", "run-fixture", "episode-fixture", "agent-fixture"),
        "branch_id": "branch-a",
        "query": text,
        "cutoff": 10,
        "corpus_generation": 5,
        "ranker_version": "lexical-v1",
        "limit": 8,
        "max_candidates": 64,
        "effect_class": "local_read_no_inference"
    })
}

#[test]
fn executable_cli_exposes_metadata_search_extract_and_preview() {
    let capabilities = run("capabilities", serde_json::Value::Null);
    assert_eq!(capabilities["product_phase"], 3);
    assert_eq!(capabilities["supported_operations"][0], "search");
    let list = run("list", serde_json::Value::Null);
    assert_eq!(list.as_array().expect("list").len(), 2);
    let search = run("search", query("settled"));
    assert_eq!(search["results"].as_array().expect("results").len(), 1);
    assert_eq!(search["inference_calls"], 0);
    let extract = run(
        "extract",
        serde_json::json!({
            "source_ids": ["hist-1"],
            "branch_id": "branch-a",
            "cutoff": 10,
            "corpus_generation": 5
        }),
    );
    assert_eq!(extract["status"], "machine_checked");
    let preview = run(
        "policy-preview",
        serde_json::json!({
            "query": query("settled"),
            "mandatory_bytes": [109, 97, 110, 100, 97, 116, 111, 114, 121],
            "phase2_prepared_manifest_sha256": sha256_hex("phase2"),
            "expires_at": "2026-09-11T12:00:00Z"
        }),
    );
    assert_eq!(preview["effect_class"], "local_preparation_only");
}

#[test]
fn executable_cli_rejects_duplicate_wire_keys() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_context-memory-cli"))
        .arg("search")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn duplicate query");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(br#"{"schema":"ascension.context-memory.query.v1","schema":"ascension.context-memory.query.v1"}"#)
        .expect("write duplicate");
    assert!(!child.wait().expect("wait").success());
}
