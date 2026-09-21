// SPDX-License-Identifier: MIT

//! Fail closed when the runtime peer contract lane stops executing a declared
//! operator-only composition test.
//!
//! Every served-composition acceptance test is `#[ignore]`d so an ordinary `cargo test`
//! needs no gateway, MCP, or harness binaries. That marker makes a missing lane step
//! indistinguishable from a passing lane: the test is simply never invoked, and a
//! regression it was written to catch stays invisible. The idle-adoption witness for the
//! permanent mid-run policy-adoption fence (issue #255) was declared that way and shipped
//! with no step, so its regression could not be observed.
//!
//! This check reads the committed lane source and the committed lane workflow, so drift in
//! either direction fails an ordinary workspace run:
//!
//! - a declared operator-only composition test that no lane step executes, and
//! - a lane `--exact` invocation that names no declared operator-only test.
//!
//! It compares text shapes; it does not start a peer. Whether the lane itself passes
//! remains the lane's own result.

const LANE_SOURCE: &str = include_str!("runtime_v4_executable_composition.rs");
const LANE_WORKFLOW: &str = include_str!("../../../.github/workflows/runtime-peer-contract.yml");

const OPERATOR_ONLY_MARKER: &str = "#[ignore = \"operator-only test; requires explicitly built gateway, MCP, and harness binaries\"]";
const LANE_TEST_BINARY: &str = "--test runtime_v4_executable_composition";
const EXACT_INVOCATION: &str = "-- --ignored --exact ";
const IDLE_ADOPTION_WITNESS: &str = "served_decision_survives_changed_policy_adopted_while_idle";

/// Operator-only tests declared in the lane source, in declaration order.
fn declared_operator_only_tests(source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut pending = false;
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed == OPERATOR_ONLY_MARKER {
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

/// Test names the lane workflow invokes with `--ignored --exact`.
fn lane_exact_targets(workflow: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in workflow.lines() {
        if !line.contains(LANE_TEST_BINARY) {
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

/// True when one lane command runs every ignored test in the binary instead of naming them.
fn lane_runs_every_ignored_test(workflow: &str) -> bool {
    workflow.lines().any(|line| {
        line.contains(LANE_TEST_BINARY)
            && line.contains("-- --ignored")
            && !line.contains("--exact")
    })
}

#[test]
fn every_declared_operator_only_composition_test_is_executed_by_the_lane() {
    let declared = declared_operator_only_tests(LANE_SOURCE);
    assert!(
        declared.contains(&IDLE_ADOPTION_WITNESS.to_owned()),
        "the lane source no longer declares the idle-adoption witness; declared: {declared:?}"
    );
    assert!(
        LANE_WORKFLOW.contains(LANE_TEST_BINARY),
        "the runtime peer contract lane never invokes {LANE_TEST_BINARY}"
    );
    let runs_every_ignored_test = lane_runs_every_ignored_test(LANE_WORKFLOW);
    let exact = lane_exact_targets(LANE_WORKFLOW);
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
    assert_eq!(declared_operator_only_tests(source), ["first_witness"]);
    assert!(declared_operator_only_tests("").is_empty());
}

#[test]
fn the_lane_check_separates_exact_targets_from_a_whole_binary_run() {
    let exact = concat!(
        "        run: cargo test --locked -j 2 --package sts2-harness ",
        "--test runtime_v4_executable_composition -- --ignored --exact witnessed_decision\n",
    );
    assert_eq!(lane_exact_targets(exact), ["witnessed_decision"]);
    assert!(!lane_runs_every_ignored_test(exact));

    let whole = concat!(
        "        run: cargo test --locked -j 2 --package sts2-harness ",
        "--test runtime_v4_executable_composition -- --ignored\n",
    );
    assert!(lane_runs_every_ignored_test(whole));
    assert!(lane_exact_targets(whole).is_empty());
}
