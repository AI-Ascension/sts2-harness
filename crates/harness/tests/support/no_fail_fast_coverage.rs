// SPDX-License-Identifier: MIT

//! The `--no-fail-fast` half of `exact_gate_lane_coverage`.
//!
//! Split into a `#[path]` sibling so this check does not push the parent over the
//! 400-nonblank preferred test budget it enforces on every other file in the tree.
//!

/// Every **workspace-wide** `cargo test` in CI must pass `--no-fail-fast`
/// (sts2-harness#645).
///
/// #645 is a reporting-honesty defect, not a flake: `cargo test` runs each test
/// target as a separate binary and aborts the whole invocation at the first
/// failing one. This crate's integration tests are individual binaries in
/// `crates/harness/tests/` and cargo runs them alphabetically by filename, so on
/// the revision this was written a single failure in `jev_bridge_process.rs`
/// (position 128) stopped the run before `served_gateway_capture_drain.rs`
/// (position 245) — 117 later binaries never executed. A green
/// `Continuous integration` therefore certified only "every binary up to the
/// first failure passed".
///
/// This asserts the **invariant**, not the literal flag text: any `cargo test`
/// that selects the whole workspace (`--workspace`, or no target selection at
/// all, which defaults to the workspace) must carry `--no-fail-fast`. A workflow
/// that drops the flag fails here; one that achieves the same effect by another
/// route passes. Deliberately narrower than "every `cargo test`": the per-lane
/// `--test <name> --exact` invocations in the peer-contract workflows each name a
/// single binary and have nothing to lose from fail-fast, so requiring the flag
/// there would be a rule with no defect behind it.
#[test]
fn every_workspace_wide_cargo_test_in_ci_keeps_running_after_a_failure() {
    let directory = super::workflows_directory();
    let mut checked = 0usize;

    for entry in std::fs::read_dir(&directory).expect("workflows directory") {
        let path = entry.expect("workflow entry").path();
        if !matches!(
            path.extension().and_then(std::ffi::OsStr::to_str),
            Some("yml" | "yaml")
        ) {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("workflow text");
        let file = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();

        for line in text.lines().filter(|line| !super::is_comment(line)) {
            let Some(command) = line.split_once("cargo test").map(|(_, rest)| rest) else {
                continue;
            };
            // Doctests are out of scope for this change, so the sibling "Run
            // doctests" step is left alone. Note that `--doc` does *not* make
            // fail-fast harmless: doctests still fail fast per crate, so a
            // `repo-policy` doctest failure suppresses every later member's.
            // The fix for that is the same flag, and it belongs in the
            // doctest step rather than here; see sts2-harness#645.
            if command.contains("--doc") {
                continue;
            }
            // A target selection narrower than the workspace cannot hide a later
            // test binary behind an earlier failure, so it is out of scope. This
            // covers the per-lane `--test <name>` invocations and the
            // other-repository `--manifest-path` ones.
            let narrowed = [
                "--test ",
                "--tests ",
                "-p ",
                "--package ",
                "--bench ",
                "--manifest-path",
                "--bin ",
            ]
            .iter()
            .any(|flag| command.contains(flag));
            let workspace_wide = command.contains("--workspace") || !narrowed;
            if !workspace_wide {
                continue;
            }
            checked += 1;
            assert!(
                command.contains("--no-fail-fast"),
                "{file}: a workspace-wide `cargo test` runs without `--no-fail-fast`, so one \
                 failing test binary silently suppresses every later binary and a green CI run \
                 cannot be read as the suite passing (sts2-harness#645). The step reads: {line}"
            );
        }
    }

    assert!(
        checked > 0,
        "the sweep found no workspace-wide `cargo test` in any workflow, so it is asserting \
         nothing (sts2-harness#645)"
    );
}
