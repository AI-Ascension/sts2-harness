// SPDX-License-Identifier: MIT

//! Fail closed when a harness lane step filters a test by name without an
//! execution gate (issues #524 and #536). A libtest filter that matches nothing
//! exits 0 and prints a success line, so `cargo test ... -- --ignored --exact
//! <name>` reports green for a renamed or removed test. `tools/exact-gate.sh`
//! closes that by asserting the match count; this check keeps the wiring honest
//! by sweeping **every** `.github/workflows/*.yml` and requiring each
//! name-filtering invocation to be guarded by that gate or to carry the one
//! documented exemption.
//!
//! The sweep is the point: the first revision enumerated three lanes through
//! three `include_str!` constants, so an unguarded invocation added to a fourth
//! file passed the whole check (#536). There is no file list here — the directory
//! is read at test time, so a workflow that does not exist yet is covered by
//! construction and the polarity is an allow-list of nothing. The one exemption
//! is not a hole: `--no-run` deliberately executes nothing, so the invocation must
//! carry the marker comment immediately above it (`--no-run` alone is not enough).
//!
//! `KNOWN_SITES` is a liveness control, not the coverage boundary: a moved count
//! fails so the change stays deliberate, while an *unguarded* site anywhere fails
//! the sweep whether or not its file is listed. This compares text shapes; it does
//! not run a lane, and the gate's own behaviour is proven by the lane's hosted run.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::PathBuf;

/// The gate as the workflows address it, at whichever checkout root a lane uses.
/// The peer lanes address `$GITHUB_WORKSPACE/harness/tools/exact-gate.sh` (they
/// check this repository out at `path: harness`); the oracle lane addresses
/// `$GITHUB_WORKSPACE/tools/exact-gate.sh`. The repository-relative
/// `tools/exact-gate.sh` is the substring both forms share.
const GATE: &str = "tools/exact-gate.sh";
/// The form the oracle lane must use; the `harness/`-prefixed path would exit 127.
const ORACLE_GATE: &str = "$GITHUB_WORKSPACE/tools/exact-gate.sh";
/// The peer form, which [`GATE`] also matches and which the oracle lane must not use.
const HARNESS_PREFIXED_GATE: &str = "harness/tools/exact-gate.sh";
const FILTER: &str = "--ignored --exact";

/// Marker the `--no-run` build gate must carry in the comment block immediately
/// above it, so a later reader does not "fix" it into an assertion on zero
/// executed tests.
const NO_RUN_EXEMPTION_MARKER: &str =
    "exempt from harness/tools/exact-gate.sh: --no-run compiles the filtered target and executes";

/// Name-filtering invocations per workflow at the revision this check was written
/// against. A workflow absent from this table must carry none.
const KNOWN_SITES: &[(&str, usize)] = &[
    ("runtime-peer-contract.yml", 13),
    ("game-information-peer-contract.yml", 4),
    ("exact-restore-conformance.yml", 1),
    ("exo-process-oracle.yml", 5),
];

/// The swept directory, as the hosted lane sees it: the harness checkout that
/// holds this crate also holds the workflows this check is about.
fn workflows_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.github/workflows")
}

/// One workflow file and its committed text.
struct LaneFile {
    name: String,
    source: String,
}

/// Every workflow file in `.github/workflows`, ordered by file name.
///
/// The directory is read at test time rather than pinned through `include_str!`,
/// because a constant list can only enumerate the files that existed when it was
/// written — which is the defect this sweep closes. A directory that cannot be
/// read fails closed instead of passing on an empty set.
fn lane_files() -> Vec<LaneFile> {
    let directory = workflows_directory();
    let entries = std::fs::read_dir(&directory)
        .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()));
    let mut files = Vec::new();
    for entry in entries {
        let path = entry.expect("workflow directory entry").path();
        let extension = path.extension().and_then(|value| value.to_str());
        if !matches!(extension, Some("yml" | "yaml")) {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .expect("workflow file name")
            .to_owned();
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        files.push(LaneFile { name, source });
    }
    files.sort_by(|left, right| left.name.cmp(&right.name));
    files
}

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
    /// Workflow file name the command was read from.
    file: String,
    line_number: usize,
    text: String,
    /// The comment block immediately above the command, if any.
    comment: String,
}

impl Invocation {
    /// True when the command is routed through the execution gate.
    fn gated(&self) -> bool {
        self.text.contains(GATE)
    }

    /// True when the command is the deliberately non-executing build gate and
    /// carries the marker that says so.
    fn exempt(&self) -> bool {
        self.text.contains("--no-run") && self.comment.contains(NO_RUN_EXEMPTION_MARKER)
    }
}

/// Every logical command in `lane` whose text filters a test by name.
///
/// A multi-line `run: |` body is folded on trailing backslashes so one logical
/// command is one entry; the gate prefix and the filter therefore always land in
/// the same entry, which is what makes the wiring checkable. The comment block
/// immediately above the first line is carried with the entry, so an exemption
/// can be required to be adjacent rather than anywhere in the file.
fn named_invocations(file: &str, lane: &str) -> Vec<Invocation> {
    let mut invocations = Vec::new();
    let mut pending: Option<Invocation> = None;
    let mut comment = String::new();
    for (index, line) in lane.lines().enumerate() {
        if pending.is_none() && is_comment(line) {
            if !comment.is_empty() {
                comment.push('\n');
            }
            comment.push_str(line.trim());
            continue;
        }
        let continued = line.trim_end().ends_with('\\');
        match pending.as_mut() {
            Some(invocation) => {
                invocation.text.push(' ');
                invocation.text.push_str(line.trim());
            }
            None => {
                pending = Some(Invocation {
                    file: file.to_owned(),
                    line_number: index + 1,
                    text: line.trim().to_owned(),
                    comment: std::mem::take(&mut comment),
                });
            }
        }
        if continued {
            continue;
        }
        comment.clear();
        let Some(finished) = pending.take() else {
            continue;
        };
        if finished.text.contains(FILTER) && !is_comment(&finished.text) {
            invocations.push(finished);
        }
    }
    invocations
}

/// Every name-filtering invocation in every swept workflow, tagged with its file.
fn sites(files: &[LaneFile]) -> Vec<Invocation> {
    let mut sites = Vec::new();
    for file in files {
        sites.extend(named_invocations(&file.name, &file.source));
    }
    sites
}

/// The named swept workflow, which must exist for the file-specific checks below.
fn file_named<'a>(files: &'a [LaneFile], name: &str) -> &'a LaneFile {
    files
        .iter()
        .find(|file| file.name == name)
        .unwrap_or_else(|| panic!("the {name} workflow must be part of the sweep"))
}

#[test]
fn every_name_filtering_invocation_in_every_workflow_is_gated_or_exempt() {
    let files = lane_files();
    assert!(
        !files.is_empty(),
        "no workflow files found under {}; this sweep cannot pass vacuously",
        workflows_directory().display()
    );

    let mut unguarded = Vec::new();
    let mut exempt = Vec::new();
    for invocation in sites(&files) {
        let where_it_is = format!("{}:{}", invocation.file, invocation.line_number);
        if invocation.gated() {
            continue;
        }
        if invocation.exempt() {
            exempt.push(invocation.file.clone());
            continue;
        }
        unguarded.push(format!(
            "{where_it_is} filters a test by name without {GATE}, so a renamed test would run \
             nothing and the step would still report green: {}",
            invocation.text
        ));
    }
    assert!(
        unguarded.is_empty(),
        "{} unguarded name-filtering invocation(s):\n{}",
        unguarded.len(),
        unguarded.join("\n")
    );
    assert_eq!(
        exempt,
        vec!["exact-restore-conformance.yml".to_owned()],
        "the only permitted ungated name-filtering invocation is the --no-run build gate, and it \
         must be the one this check was written against"
    );
}

/// Every site the sweep finds must be one this check was written against, and an
/// unguarded site in a file outside the table must still fail the sweep.
///
/// The table is a liveness control: it fails when an existing site moves or a
/// workflow gains one, so the change is deliberate. The sweep is the boundary: it
/// is what makes a new file's unguarded site fail rather than pass 4/4 (#536).
#[test]
fn the_swept_site_table_matches_the_workflows_and_covers_every_file() {
    let files = lane_files();
    let mut counts: Vec<(String, usize)> = Vec::new();
    for invocation in sites(&files) {
        match counts.iter_mut().find(|(name, _)| name == &invocation.file) {
            Some((_, count)) => *count += 1,
            None => counts.push((invocation.file.clone(), 1)),
        }
    }
    let mut expected: Vec<(String, usize)> = KNOWN_SITES
        .iter()
        .map(|(name, count)| ((*name).to_owned(), *count))
        .filter(|(_, count)| *count > 0)
        .collect();
    counts.sort_by(|left, right| left.0.cmp(&right.0));
    expected.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(
        counts, expected,
        "the name-filtering invocations moved relative to the table this check was written \
         against; update KNOWN_SITES deliberately"
    );
}

/// The oracle lane checks out at the repository root, so its gate must be
/// addressed at that root and **not** through the `harness/`-prefixed form the
/// other lanes use: that path does not exist on its runner and every leg would
/// exit 127. The sweep's [`GATE`] substring matches both forms, so this is the
/// check that pins *which* root this lane uses. The pin is **per leg**: a
/// file-level `contains` stays satisfied while one leg keeps the exact form and
/// another points anywhere else containing [`GATE`], so the loop below repeats it.
#[test]
fn the_oracle_lane_addresses_the_gate_at_its_own_checkout_root() {
    let files = lane_files();
    let oracle = file_named(&files, "exo-process-oracle.yml");
    assert!(
        oracle.source.contains(ORACLE_GATE),
        "exo-process-oracle.yml checks out at the repository root; it must address the gate as \
         {ORACLE_GATE}"
    );
    assert!(
        !oracle.source.contains(HARNESS_PREFIXED_GATE),
        "exo-process-oracle.yml checks out at the repository root; addressing the gate as \
         {HARNESS_PREFIXED_GATE} would exit 127 on every leg. Use {ORACLE_GATE} instead."
    );
    for invocation in named_invocations(&oracle.name, &oracle.source) {
        assert!(
            invocation.text.contains(ORACLE_GATE),
            "exo-process-oracle.yml line {} must address the gate as {ORACLE_GATE} exactly, \
             not any other path containing {GATE}: {}",
            invocation.line_number,
            invocation.text
        );
    }
}

/// Each oracle leg must name the test it filters.
///
/// `--exact` with no positional filter does not witness a specific test: in that
/// form libtest runs every filtered test in the target, so a renamed oracle
/// would still execute under its new name and the leg would stay green -- the
/// #524 class this gate exists to close. The oracle lane always writes the name
/// after the filter, so requiring a non-flag token there is exact for this lane;
/// the other lanes also use a leading-filter form (`<name> -- --ignored --exact`),
/// which this assertion deliberately does not constrain.
#[test]
fn every_oracle_leg_names_the_test_it_filters() {
    let files = lane_files();
    let oracle = file_named(&files, "exo-process-oracle.yml");
    for invocation in named_invocations(&oracle.name, &oracle.source) {
        let text = invocation.text.as_str();
        let Some((_, after)) = text.split_once(FILTER) else {
            continue;
        };
        let name = after.split_whitespace().next();
        assert!(
            matches!(name, Some(token) if !token.starts_with('-')),
            "exo-process-oracle.yml line {} filters with {FILTER} but names no test after it; \
             an unnamed filter runs every ignored test in the target and so cannot witness that \
             the named oracle still exists: {}",
            invocation.line_number,
            text
        );
    }
}

/// The named `--no-run` site must stay ungated, must execute nothing, and must
/// keep the adjacent marker that says why.
///
/// It is asserted by name rather than through the sweep so that removing its
/// marker — which is what makes a later reader "fix" it into a match assertion on
/// zero executed tests — fails here, and so that the exemption cannot silently
/// migrate to another invocation.
#[test]
fn the_no_run_build_gate_is_exempt_and_explains_itself() {
    let files = lane_files();
    let exact_restore = files
        .iter()
        .find(|file| file.name == "exact-restore-conformance.yml")
        .expect("the exact-restore lane must exist");
    let invocations = named_invocations(&exact_restore.name, &exact_restore.source);
    let [invocation] = invocations.as_slice() else {
        panic!(
            "the exact-restore lane must carry exactly one name-filtering invocation, found {}",
            invocations.len()
        );
    };
    assert!(
        invocation.text.contains("--no-run"),
        "the exact-restore lane gained a name-filtering invocation that executes a test; it \
         must be guarded by {GATE}: {}",
        invocation.text
    );
    assert!(
        !invocation.gated(),
        "the --no-run build gate must not be guarded: nothing executes there, so a match \
         assertion would fail a healthy build"
    );
    assert!(
        invocation.exempt(),
        "the --no-run build gate must carry the marker comment immediately above it so a later \
         reader does not turn it into an assertion on zero executed tests"
    );
}

/// A gate prefix and a bare command must not be confused, including across a
/// folded multi-line body and against the two checkout roots the lanes use.
/// A comment that mentions the filter must not be counted as an invocation.
#[test]
fn the_coverage_check_folds_continuations_and_requires_a_gate_prefix() {
    let gated_at_harness_root = concat!(
        "        run: |\n",
        "          $GITHUB_WORKSPACE/harness/tools/exact-gate.sh - cargo test --locked --package sts2-harness \\\n",
        "            --test runtime_v4_executable_composition -- --ignored --exact witness\n",
    );
    let gated_at_repository_root = concat!(
        "        run: |\n",
        "          \"$GITHUB_WORKSPACE/tools/exact-gate.sh\" - cargo test --locked --manifest-path \\\n",
        "            experiments/exo-agent/bridge/Cargo.toml --test process_oracle -- --ignored --exact \\\n",
        "            real_exo_process_matrix\n",
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
    let comment_quoting_the_filter = concat!(
        "          # The four evidence logs are written by tools/exact-gate.sh and the\n",
        "          # --ignored --exact counter is what makes a dropped step visible.\n",
    );

    for (label, sample) in [
        ("harness", gated_at_harness_root),
        ("repository", gated_at_repository_root),
    ] {
        let invocations = named_invocations("sample.yml", sample);
        assert_eq!(
            invocations.len(),
            1,
            "the {label}-root gated sample must fold once"
        );
        let Some(invocation) = invocations.first() else {
            continue;
        };
        assert!(invocation.gated(), "the {label} sample must read as gated");
    }

    let bare_invocations = named_invocations("sample.yml", bare_command);
    assert_eq!(bare_invocations.len(), 1, "the bare sample must fold once");
    let Some(bare_invocation) = bare_invocations.first() else {
        return;
    };
    assert!(!bare_invocation.gated());

    assert!(named_invocations("sample.yml", unfiltered).is_empty());
    assert!(named_invocations("sample.yml", comment_quoting_the_filter).is_empty());
}
