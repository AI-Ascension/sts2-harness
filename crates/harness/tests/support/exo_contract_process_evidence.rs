// SPDX-License-Identifier: MIT

use serde_json::Value;
use sts2_harness::{EXO_SOURCE_REVISION, exo_bridge_manifest, sha256_hex};

const RECORD: &str =
    include_str!("../../../../docs/evidence/exo-executor-process-oracle-20260915.json");
const EXTENSION: &[u8] = include_bytes!("../../../../experiments/exo-agent/extension/src/index.ts");
const ORACLE: &[u8] =
    include_bytes!("../../../../experiments/exo-agent/bridge/tests/process_oracle.rs");

const RECORD_PATH: &str = "docs/evidence/exo-executor-process-oracle-20260915.json";
const ACCEPTED_DECISIONS: [&str; 4] = ["action", "plan", "wait", "reobserve"];
const MODEL_OUTPUT_REJECTIONS: [&str; 9] = [
    "illegal_action",
    "multiple_json",
    "truncated_json",
    "empty_output",
    "oversized_output",
    "unknown_field",
    "refusal",
    "tool_escalation",
    "multiple_messages",
];
const RETRY_CONTAINMENT: [&str; 2] = ["429_one_egress", "500_one_egress"];
const PRE_MODEL_REJECTIONS: [&str; 12] = [
    "describe",
    "wrong_revision",
    "unsupported_map",
    "wrong_generation",
    "unknown_field_input",
    "unsupported_expert",
    "invalid_utf8",
    "oversized_input",
    "duplicate_field_input",
    "missing_eof",
    "package_digest_mismatch",
    "executor_nonzero",
];

fn record() -> Value {
    serde_json::from_str(RECORD).expect("process evidence record is JSON")
}

#[test]
fn recorded_process_evidence_matches_shipped_extension_and_oracle_bytes() {
    let record = record();
    assert_eq!(
        record["schema"].as_str(),
        Some("sts2.exo-one-shot-process-evidence-v1")
    );
    assert_eq!(
        record["evidence"].as_str(),
        Some("real-pinned-Exo-with-synthetic-model-no-game")
    );
    assert_eq!(record["exo_revision"].as_str(), Some(EXO_SOURCE_REVISION));
    assert_eq!(record["full_runtime_admission"].as_bool(), Some(false));
    assert_eq!(
        record["extension_sha256"].as_str(),
        Some(sha256_hex(EXTENSION).as_str()),
        "the recorded real-process run must describe the shipped extension bytes"
    );
    assert_eq!(
        record["oracle_sha256"].as_str(),
        Some(sha256_hex(ORACLE).as_str()),
        "the recorded run must have been produced by the committed oracle source"
    );
}

#[test]
fn recorded_process_evidence_covers_every_case_group() {
    let record = record();
    let cases = record["cases"].as_array().expect("recorded cases");
    let mut names = Vec::new();
    for case in cases {
        let name = case["case"].as_str().expect("case name");
        assert_eq!(
            case["passed"].as_bool(),
            Some(true),
            "recorded case {name} did not pass"
        );
        let requests = case["model_requests"].as_u64().expect("model requests");
        if PRE_MODEL_REJECTIONS.contains(&name) {
            assert_eq!(
                requests, 0,
                "pre-model rejection {name} contacted the model"
            );
        } else {
            assert_eq!(requests, 1, "case {name} did not make exactly one egress");
        }
        names.push(name.to_owned());
    }
    let mut expected = Vec::new();
    expected.extend(ACCEPTED_DECISIONS);
    expected.extend(MODEL_OUTPUT_REJECTIONS);
    expected.extend(RETRY_CONTAINMENT);
    expected.extend(PRE_MODEL_REJECTIONS);
    expected.sort_unstable();
    names.sort_unstable();
    assert_eq!(names, expected);
    for name in RETRY_CONTAINMENT {
        let case = cases
            .iter()
            .find(|case| case["case"].as_str() == Some(name))
            .expect("retry case");
        assert_eq!(case["evidence"]["fetch_attempts"].as_u64(), Some(3));
        assert_eq!(case["evidence"]["forwarded_requests"].as_u64(), Some(1));
        assert_eq!(case["evidence"]["denied_requests"].as_u64(), Some(2));
    }
}

#[test]
fn artifact_manifest_records_the_same_process_evidence() {
    let manifest: Value =
        serde_json::from_str(exo_bridge_manifest()).expect("artifact manifest is JSON");
    let recorded = &manifest["process_evidence"];
    assert_eq!(recorded["record"].as_str(), Some(RECORD_PATH));
    assert_eq!(recorded["exo_revision"].as_str(), Some(EXO_SOURCE_REVISION));
    assert_eq!(
        recorded["extension_sha256"].as_str(),
        Some(sha256_hex(EXTENSION).as_str())
    );
    assert_eq!(
        recorded["oracle_sha256"].as_str(),
        Some(sha256_hex(ORACLE).as_str())
    );
    assert_eq!(
        recorded["runtime_admission"].as_str(),
        Some("source-process-only")
    );
    assert_eq!(recorded["native_game"].as_str(), Some("unverified"));
}
