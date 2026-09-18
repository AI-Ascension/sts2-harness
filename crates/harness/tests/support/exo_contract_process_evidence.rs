// SPDX-License-Identifier: MIT

use serde_json::Value;
use sts2_harness::{EXO_SOURCE_REVISION, exo_bridge_manifest, sha256_hex};

const RECORD: &str =
    include_str!("../../../../docs/evidence/exo-executor-process-oracle-20260917.json");
const EXTENSION: &[u8] = include_bytes!("../../../../experiments/exo-agent/extension/src/index.ts");
const ORACLE: &[u8] =
    include_bytes!("../../../../experiments/exo-agent/bridge/tests/process_oracle.rs");
const SUPPORT: [(&str, &[u8]); 2] = [
    (
        "tests/support/mod.rs",
        include_bytes!("../../../../experiments/exo-agent/bridge/tests/support/mod.rs"),
    ),
    (
        "tests/support/projection.rs",
        include_bytes!("../../../../experiments/exo-agent/bridge/tests/support/projection.rs"),
    ),
];

const RECORD_PATH: &str = "docs/evidence/exo-executor-process-oracle-20260917.json";
const ADVERTISED_RECORD: &str =
    include_str!("../../../../docs/evidence/exo-advertised-variant-negatives-20260918.json");
const ADVERTISED_ORACLE: &[u8] =
    include_bytes!("../../../../experiments/exo-agent/bridge/tests/advertised_variant_oracle.rs");
const ADVERTISED_RECORD_PATH: &str = "docs/evidence/exo-advertised-variant-negatives-20260918.json";
const ACCEPTED_DECISIONS: [&str; 4] = ["action", "plan", "wait", "reobserve"];
const ADVERTISED_PROBES: [&str; 5] = [
    "describe",
    "describe_repeated",
    "map_refused_pre_inference",
    "management_refused_pre_inference",
    "tampered_config_rejected",
];
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
const EMPTY_TOOL_REQUEST: [&str; 1] = ["request_tools_are_empty"];
/// One case per forbidden name/alias/variant; each probes the bridge and the executor boundary,
/// so it makes exactly two egresses and must carry both typed codes.
const FORBIDDEN_TOOL_BY_NAME: [&str; 12] = [
    "forbidden_tool_by_name_shell",
    "forbidden_tool_by_name_shell_upper",
    "forbidden_tool_by_name_shell_mixed",
    "forbidden_tool_by_name_shell_functions_namespace",
    "forbidden_tool_by_name_shell_builtin_namespace",
    "forbidden_tool_by_name_install_agent_tool",
    "forbidden_tool_by_name_uninstall_agent_tool",
    "forbidden_tool_by_name_manage_tool",
    "forbidden_tool_by_name_inspect_tools",
    "forbidden_tool_by_name_install_skill",
    "forbidden_tool_by_name_remember",
    "forbidden_tool_by_name_lookup_query",
];
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
    for (path, bytes) in SUPPORT {
        assert_eq!(
            record["support_sha256"][path].as_str(),
            Some(sha256_hex(bytes).as_str()),
            "the recorded run must have been produced by the committed oracle support {path}"
        );
    }
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
        } else if FORBIDDEN_TOOL_BY_NAME.contains(&name) {
            assert_eq!(requests, 2, "case {name} must probe both boundaries once");
            assert_eq!(
                case["evidence"]["executor_error_code"].as_str(),
                Some("exo_forbidden_tool"),
                "case {name} lacks the typed executor denial"
            );
            assert_eq!(
                case["evidence"]["bridge_error_code"].as_str(),
                Some("exo_bridge_executor_failed"),
                "case {name} lacks the fail-closed bridge code"
            );
            assert_eq!(
                case["evidence"]["executor_decision_absent"].as_bool(),
                Some(true)
            );
            assert_eq!(
                case["evidence"]["bridge"]["forwarded_requests"].as_u64(),
                Some(1)
            );
        } else {
            assert_eq!(requests, 1, "case {name} did not make exactly one egress");
        }
        if EMPTY_TOOL_REQUEST.contains(&name) {
            assert_eq!(case["tools_advertised"].as_u64(), Some(0));
        }
        names.push(name.to_owned());
    }
    let mut expected = Vec::new();
    expected.extend(ACCEPTED_DECISIONS);
    expected.extend(MODEL_OUTPUT_REJECTIONS);
    expected.extend(RETRY_CONTAINMENT);
    expected.extend(EMPTY_TOOL_REQUEST);
    expected.extend(FORBIDDEN_TOOL_BY_NAME);
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
    for (path, bytes) in SUPPORT {
        assert_eq!(
            recorded["support_sha256"][path].as_str(),
            Some(sha256_hex(bytes).as_str())
        );
    }
    assert_eq!(
        recorded["runtime_admission"].as_str(),
        Some("source-process-only")
    );
    assert_eq!(recorded["native_game"].as_str(), Some("unverified"));
}

/// The advertised-variant record is a separate real-process run with its own oracle source, so it
/// needs its own byte check: a change to the shipped extension or to that oracle without a refreshed
/// run must fail rather than leave a stale record claiming a run that no longer describes the code.
#[test]
fn recorded_advertised_variant_evidence_matches_shipped_bytes() {
    let record: Value =
        serde_json::from_str(ADVERTISED_RECORD).expect("advertised-variant record is JSON");
    assert_eq!(
        record["schema"].as_str(),
        Some("sts2.exo-advertised-variant-evidence-v1")
    );
    assert_eq!(
        record["evidence"].as_str(),
        Some("real-process-synthetic-model-no-game")
    );
    assert_eq!(record["exo_revision"].as_str(), Some(EXO_SOURCE_REVISION));
    assert_eq!(record["full_runtime_admission"].as_bool(), Some(false));
    assert_eq!(
        record["extension_sha256"].as_str(),
        Some(sha256_hex(EXTENSION).as_str()),
        "the advertised-variant run must describe the shipped extension bytes"
    );
    assert_eq!(
        record["oracle_sha256"].as_str(),
        Some(sha256_hex(ADVERTISED_ORACLE).as_str()),
        "the advertised-variant run must have been produced by the committed oracle source"
    );
    // The whole point of this record is that no probe reached a model.
    assert_eq!(record["model_requests"].as_u64(), Some(0));
    let probes = record["probes"].as_array().expect("recorded probes");
    let mut names = probes
        .iter()
        .map(|probe| probe.as_str().expect("probe name").to_owned())
        .collect::<Vec<_>>();
    let mut expected = ADVERTISED_PROBES.to_vec();
    expected.sort_unstable();
    names.sort_unstable();
    assert_eq!(names, expected);
}

#[test]
fn artifact_manifest_records_the_same_advertised_variant_evidence() {
    let manifest: Value =
        serde_json::from_str(exo_bridge_manifest()).expect("artifact manifest is JSON");
    let recorded = &manifest["process_evidence"]["advertised_variant_evidence"];
    assert_eq!(recorded["record"].as_str(), Some(ADVERTISED_RECORD_PATH));
    assert_eq!(recorded["exo_revision"].as_str(), Some(EXO_SOURCE_REVISION));
    assert_eq!(
        recorded["extension_sha256"].as_str(),
        Some(sha256_hex(EXTENSION).as_str())
    );
    assert_eq!(
        recorded["oracle_sha256"].as_str(),
        Some(sha256_hex(ADVERTISED_ORACLE).as_str())
    );
    assert_eq!(recorded["model_requests"].as_u64(), Some(0));
    assert_eq!(
        recorded["runtime_admission"].as_str(),
        Some("source-process-only")
    );
    assert_eq!(recorded["native_game"].as_str(), Some("unverified"));
}
