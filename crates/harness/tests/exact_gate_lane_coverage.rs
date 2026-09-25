// SPDX-License-Identifier: MIT

//! Fail closed when a harness lane step filters a test by name without an
//! execution gate (issue #524).
//!
//! A libtest filter that matches nothing exits 0 and prints a success line, so
//! `cargo test ... -- --ignored --exact <name>` reports green for a renamed or
//! removed test. `tools/exact-gate.sh` closes that by asserting the match count.
//! This check keeps the wiring honest: every invocation in the three
//! name-filtering lanes must either be guarded by that gate or be exempt.
//!
//! The one exemption is not a hole. `exact-restore-conformance.yml` compiles a
//! filtered target with `--no-run` and deliberately executes nothing, so a match
//! assertion there would fail a healthy build; it carries a source comment
//! instead, and this check requires that comment to be present.
//!
//! It compares text shapes; it does not run a lane, a compiler, or a test
//! binary. The gate's own behaviour is proven by the lane's hosted run.

const RUNTIME_LANE: &str = include_str!("../../../.github/workflows/runtime-peer-contract.yml");
const GAME_INFORMATION_LANE: &str =
    include_str!("../../../.github/workflows/game-information-peer-contract.yml");
const EXACT_RESTORE_LANE: &str =
    include_str!("../../../.github/workflows/exact-restore-conformance.yml");

/// The gate as the workflows address it. Every harness lane checks this
/// repository out at `path: harness`, so the gate's checkout-rooted path is
/// `$GITHUB_WORKSPACE/harness/tools/exact-gate.sh` — not the repository-relative
/// one, which does not exist on a runner.
const GATE: &str = "harness/tools/exact-gate.sh";
const FILTER: &str = "--ignored --exact";

/// Marker the `--no-run` build gate must carry so a later reader does not "fix"
/// it into an assertion on zero executed tests.
const NO_RUN_EXEMPTION_MARKER: &str =
    "exempt from harness/tools/exact-gate.sh: --no-run compiles the filtered target and executes";

/// True when a folded command is prose rather than something the shell will run.
///
/// The exemption comment names the gate, and one lane comment quotes the filter to
/// explain an earlier pipeline defect; neither is an invocation, and counting them
/// would make the check report a phantom unguarded step.
fn is_comment(text: &str) -> bool {
    text.trim_start().starts_with('#')
}

/// One logical shell command, reconstructed from its line continuations.
struct Invocation {
    line_number: usize,
    text: String,
}

/// Every logical command in `lane` whose text filters a test by name.
///
/// A multi-line `run: |` body is folded on trailing backslashes so one logical
/// command is one entry; the gate prefix and the filter therefore always land in
/// the same entry, which is what makes the wiring checkable.
fn named_invocations(lane: &str) -> Vec<Invocation> {
    let mut invocations = Vec::new();
    let mut pending: Option<Invocation> = None;
    for (index, line) in lane.lines().enumerate() {
        let continued = line.trim_end().ends_with('\\');
        match pending.as_mut() {
            Some(invocation) => {
                invocation.text.push(' ');
                invocation.text.push_str(line.trim());
            }
            None => {
                pending = Some(Invocation {
                    line_number: index + 1,
                    text: line.trim().to_owned(),
                });
            }
        }
        if continued {
            continue;
        }
        let Some(finished) = pending.take() else {
            continue;
        };
        if finished.text.contains(FILTER) && !is_comment(&finished.text) {
            invocations.push(finished);
        }
    }
    invocations
}

fn gated(invocation: &Invocation) -> bool {
    invocation.text.contains(GATE)
}

fn assert_all_gated(name: &str, lane: &str, expected: usize) {
    let invocations = named_invocations(lane);
    assert_eq!(
        invocations.len(),
        expected,
        "{name} no longer carries the {expected} name-filtering invocations this check was \
         written against; update the count deliberately"
    );
    for invocation in &invocations {
        assert!(
            gated(invocation),
            "{name} line {} filters a test by name without {GATE}, so a renamed test would run \
             nothing and the step would still report green: {}",
            invocation.line_number,
            invocation.text
        );
    }
}

#[test]
fn every_invocation_in_the_runtime_lane_is_gated() {
    assert_all_gated("runtime-peer-contract.yml", RUNTIME_LANE, 13);
}

#[test]
fn every_invocation_in_the_game_information_lane_is_gated() {
    assert_all_gated(
        "game-information-peer-contract.yml",
        GAME_INFORMATION_LANE,
        4,
    );
}

/// The `--no-run` build gate must stay ungated and must say why.
#[test]
fn the_no_run_build_gate_is_exempt_and_explains_itself() {
    let invocations = named_invocations(EXACT_RESTORE_LANE);
    assert_eq!(
        invocations.len(),
        1,
        "the exact-restore lane invocation count changed"
    );
    let [invocation] = invocations.as_slice() else {
        return;
    };
    assert!(
        invocation.text.contains("--no-run"),
        "the exact-restore lane gained a name-filtering invocation that executes a test; it \
         must be guarded by {GATE}: {}",
        invocation.text
    );
    assert!(
        !gated(invocation),
        "the --no-run build gate must not be guarded: nothing executes there, so a match \
         assertion would fail a healthy build"
    );
    assert!(
        EXACT_RESTORE_LANE.contains(NO_RUN_EXEMPTION_MARKER),
        "the --no-run build gate must carry its exemption comment so a later reader does not \
         turn it into an assertion on zero executed tests"
    );
}

/// A gate prefix and a bare command must not be confused, including across a
/// folded multi-line body: that is the shape the lanes use.
#[test]
fn the_coverage_check_folds_continuations_and_requires_a_gate_prefix() {
    let gated_command = concat!(
        "        run: |\n",
        "          $GITHUB_WORKSPACE/harness/tools/exact-gate.sh - cargo test --locked --package sts2-harness \\\n",
        "            --test runtime_v4_executable_composition -- --ignored --exact witness\n",
    );
    let bare_command = concat!(
        "        run: |\n",
        "          cargo test --locked --package sts2-harness \\\n",
        "            --test runtime_v4_executable_composition -- --ignored --exact witness\n",
    );
    let unfiltered = concat!(
        "        run: |\n",
        "          $GITHUB_WORKSPACE/harness/tools/exact-gate.sh - cargo build --locked --package sts2-harness\n",
    );

    let gated_invocations = named_invocations(gated_command);
    assert_eq!(
        gated_invocations.len(),
        1,
        "the gated sample must fold once"
    );
    let Some(gated_invocation) = gated_invocations.first() else {
        return;
    };
    assert!(gated(gated_invocation));

    let bare_invocations = named_invocations(bare_command);
    assert_eq!(bare_invocations.len(), 1, "the bare sample must fold once");
    let Some(bare_invocation) = bare_invocations.first() else {
        return;
    };
    assert!(!gated(bare_invocation));

    assert!(named_invocations(unfiltered).is_empty());
}
