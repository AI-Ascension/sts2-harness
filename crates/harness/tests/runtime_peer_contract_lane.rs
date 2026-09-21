// SPDX-License-Identifier: MIT

//! Fail closed when the runtime peer contract lane stops executing a declared
//! operator-only composition test.
//!
//! Every served-composition acceptance test is `#[ignore]`d so an ordinary `cargo test`
//! needs no gateway, MCP, or harness binaries. That marker makes a missing lane step
//! indistinguishable from a passing lane: the test is simply never invoked, and a
//! regression it was written to catch stays invisible. The idle-adoption witness for the
//! permanent mid-run policy-adoption fence (issue #255) was declared that way and shipped
//! with no step, so its regression could not be observed; the executable REST selector
//! composition (issue #148) was declared the same way and reached no gate at all.
//!
//! This check reads the committed lane sources and the committed lane workflow, so drift in
//! either direction fails an ordinary workspace run:
//!
//! - a declared operator-only composition test that no lane step executes, and
//! - a lane `--exact` invocation that names no declared operator-only test.
//!
//! Each lane is paired with the test binary its declarations belong to and with the operator
//! marker that binary declares them under, so one binary's step is never compared against
//! another binary's declarations, and a lane never inherits a sibling's marker. The shipped
//! host-lease campaign downstream is the third lane: its witness runs the operator-built
//! `synthetic_mod_server` binary through the environment-configured sideband, and no step
//! invoked it before.
//!
//! It compares text shapes; it does not start a peer. Whether the lane itself passes
//! remains the lane's own result.

const LANE_WORKFLOW: &str = include_str!("../../../.github/workflows/runtime-peer-contract.yml");

/// Marker used by the served compositions that need the built gateway, MCP, and harness binaries.
const SERVED_COMPOSITION_MARKER: &str = "#[ignore = \"operator-only test; requires explicitly built gateway, MCP, and harness binaries\"]";
/// Marker used by the served host-lease terminal for the operator-built campaign downstream.
const HOST_LEASE_CAMPAIGN_MARKER: &str =
    "#[ignore = \"operator-only: requires the built synthetic_mod_server operator binary\"]";
const EXACT_INVOCATION: &str = "-- --ignored --exact ";

/// One declared operator-only lane: its source, its test binary, and one witness it must keep.
struct Lane {
    source: &'static str,
    test_binary: &'static str,
    marker: &'static str,
    required_witness: &'static str,
}

const LANES: &[Lane] = &[
    Lane {
        source: include_str!("runtime_v4_executable_composition.rs"),
        test_binary: "--test runtime_v4_executable_composition",
        marker: SERVED_COMPOSITION_MARKER,
        required_witness: "served_decision_survives_changed_policy_adopted_while_idle",
    },
    Lane {
        source: include_str!("runtime_v4_rest_executable_composition.rs"),
        test_binary: "--test runtime_v4_rest_executable_composition",
        marker: SERVED_COMPOSITION_MARKER,
        required_witness: "executable_runtime_v4_rest_composes_full_selection_recovery_chain",
    },
    Lane {
        source: include_str!("host_lease_control_served.rs"),
        test_binary: "--test host_lease_control_served",
        marker: HOST_LEASE_CAMPAIGN_MARKER,
        required_witness: "the_env_configured_campaign_downstream_answers_a_signed_install",
    },
];

/// Operator-only tests declared in a lane source, in declaration order.
fn declared_operator_only_tests(source: &str, marker: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut pending = false;
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed == marker {
            pending = true;
            continue;
        }
        if !pending {
            continue;
        }
        if let Some(name) = attributed_test_name(trimmed) {
            names.push(name);
            pending = false;
        }
    }
    names
}

fn attributed_test_name(trimmed: &str) -> Option<String> {
    let rest = trimmed.strip_prefix("fn ")?;
    let (name, _) = rest.split_once('(')?;
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return None;
    }
    Some(name.to_owned())
}

/// Test names the lane workflow invokes for `test_binary` with `--ignored --exact`.
fn lane_exact_targets(workflow: &str, test_binary: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in workflow.lines() {
        if !line.contains(test_binary) {
            continue;
        }
        if let Some((_, tail)) = line.split_once(EXACT_INVOCATION)
            && let Some(name) = tail.split_whitespace().next()
        {
            names.push(name.to_owned());
        }
    }
    names
}

/// True when one lane command runs every ignored test in `test_binary` instead of naming them.
fn lane_runs_every_ignored_test(workflow: &str, test_binary: &str) -> bool {
    workflow.lines().any(|line| {
        line.contains(test_binary) && line.contains("-- --ignored") && !line.contains("--exact")
    })
}

#[test]
fn every_declared_operator_only_composition_test_is_executed_by_the_lane() {
    for lane in LANES {
        let declared = declared_operator_only_tests(lane.source, lane.marker);
        assert!(
            declared
                .iter()
                .any(|name| name.as_str() == lane.required_witness),
            "the lane source no longer declares {} under its own marker; declared: {declared:?}",
            lane.required_witness
        );
        assert!(
            LANE_WORKFLOW.contains(lane.test_binary),
            "the runtime peer contract lane never invokes {}",
            lane.test_binary
        );
        let runs_every_ignored_test = lane_runs_every_ignored_test(LANE_WORKFLOW, lane.test_binary);
        let exact = lane_exact_targets(LANE_WORKFLOW, lane.test_binary);
        for name in &declared {
            assert!(
                runs_every_ignored_test || exact.contains(name),
                "operator-only test {name} is declared but no runtime peer contract step runs it"
            );
        }
        for target in &exact {
            assert!(
                declared.contains(target),
                "the lane runs {target} but the source declares no such operator-only test"
            );
        }
    }
}

#[test]
fn the_lane_check_reads_declared_tests_and_ignores_unrelated_ignores() {
    let source = concat!(
        "#[test]\n",
        "#[ignore = \"operator-only test; requires explicitly built gateway, MCP, and harness binaries\"]\n",
        "fn first_witness() -> Result<(), Box<dyn std::error::Error>> {\n",
        "    Ok(())\n",
        "}\n",
        "#[test]\n",
        "#[ignore = \"launched explicitly by another test\"]\n",
        "fn helper_child() {}\n",
    );
    assert_eq!(
        declared_operator_only_tests(source, SERVED_COMPOSITION_MARKER),
        ["first_witness"]
    );
    assert!(declared_operator_only_tests("", SERVED_COMPOSITION_MARKER).is_empty());
}

/// A lane must not read another lane's marker: the host-lease campaign downstream declares its
/// witness under a marker that names its own operator binary, not the peer binaries.
#[test]
fn the_lane_check_reads_only_its_own_operator_marker() {
    let source = concat!(
        "#[test]\n",
        "#[ignore = \"operator-only: requires the built synthetic_mod_server operator binary\"]\n",
        "fn campaign_witness() -> Result<(), Box<dyn std::error::Error>> {\n",
        "    Ok(())\n",
        "}\n",
        "#[test]\n",
        "#[ignore = \"operator-only test; requires explicitly built gateway, MCP, and harness binaries\"]\n",
        "fn peer_witness() {}\n",
    );
    assert_eq!(
        declared_operator_only_tests(source, HOST_LEASE_CAMPAIGN_MARKER),
        ["campaign_witness"]
    );
    assert_eq!(
        declared_operator_only_tests(source, SERVED_COMPOSITION_MARKER),
        ["peer_witness"]
    );
}

#[test]
fn the_lane_check_separates_exact_targets_from_a_whole_binary_run() {
    let binary = "--test runtime_v4_executable_composition";
    let exact = concat!(
        "        run: cargo test --locked -j 2 --package sts2-harness ",
        "--test runtime_v4_executable_composition -- --ignored --exact witnessed_decision\n",
    );
    assert_eq!(lane_exact_targets(exact, binary), ["witnessed_decision"]);
    assert!(!lane_runs_every_ignored_test(exact, binary));

    let whole = concat!(
        "        run: cargo test --locked -j 2 --package sts2-harness ",
        "--test runtime_v4_executable_composition -- --ignored\n",
    );
    assert!(lane_runs_every_ignored_test(whole, binary));
    assert!(lane_exact_targets(whole, binary).is_empty());
}

/// A step for one test binary must never be read as a step for its sibling, whose file stem
/// merely shares a prefix.
#[test]
fn the_lane_check_never_cross_reads_a_sibling_test_binary() {
    let workflow = concat!(
        "        run: cargo test --locked -j 2 --package sts2-harness ",
        "--test runtime_v4_executable_composition -- --ignored --exact witnessed_decision\n",
        "        run: cargo test --locked -j 2 --package sts2-harness ",
        "--test runtime_v4_rest_executable_composition -- --ignored --exact rest_witnessed_decision\n",
    );
    assert_eq!(
        lane_exact_targets(workflow, "--test runtime_v4_executable_composition"),
        ["witnessed_decision"]
    );
    assert_eq!(
        lane_exact_targets(workflow, "--test runtime_v4_rest_executable_composition"),
        ["rest_witnessed_decision"]
    );
    assert!(!lane_runs_every_ignored_test(
        workflow,
        "--test runtime_v4_rest_executable_composition"
    ));
}
