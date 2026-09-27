// SPDX-License-Identifier: MIT

//! Tests for the `CONFLICT001` conflict-marker rule, including the #638
//! byte-level regression cases.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::{MARKER_RUN, check_markers, conflict_marker, findings};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn reported(text: &str) -> Vec<String> {
    let mut findings = Vec::new();
    check_markers("CHANGELOG.md", text.as_bytes(), &mut findings);
    findings
        .iter()
        .map(|finding| format!("{} {}", finding.rule, finding.message))
        .collect()
}

/// The gate is worthless if it misses a form, so all three are named
/// explicitly rather than generated: a reader must be able to see that the
/// opener, the separator and the closer are each covered.
#[test]
fn identifies_the_opener_the_separator_and_the_closer() {
    assert_eq!(conflict_marker("<<<<<<< HEAD"), Some('<'));
    assert_eq!(conflict_marker("======="), Some('='));
    assert_eq!(conflict_marker(">>>>>>> topic"), Some('>'));
}

/// The exact state `main` shipped: a bare `=======` with no opening or
/// closing counterpart, between two changelog entries. Reporting the line
/// number is what makes the finding remediable in review.
#[test]
fn reports_a_bare_separator_left_by_a_three_way_merge() {
    let findings = reported("- **First entry.** Text.\n=======\n- **Second entry.** Text.\n");
    assert_eq!(findings.len(), 1, "expected one finding: {findings:?}");
    assert!(
        findings[0].contains("line 2"),
        "unexpected finding: {findings:?}"
    );
}

/// A complete conflict block, the shape a rebase actually leaves behind.
/// All three forms fire, because the separator alone is the case that got
/// through and a gate that only matched triples would miss it.
#[test]
fn reports_every_line_of_a_complete_conflict_block() {
    let findings = reported("<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> feature\n");
    assert_eq!(findings.len(), 3, "expected three findings: {findings:?}");
    for (position, expected) in ["line 1", "line 3", "line 5"].iter().enumerate() {
        assert!(
            findings[position].contains(expected),
            "finding {position} should name {expected}: {findings:?}"
        );
    }
}

/// Each form reported on its own, so a regression in one cannot hide behind
/// the other two in the block test above.
#[test]
fn reports_the_opener_the_separator_and_the_closer_each_on_their_own() {
    for (marker, findings) in [
        ('<', reported("<<<<<<< HEAD\n")),
        ('=', reported("=======\n")),
        ('>', reported(">>>>>>> topic\n")),
    ] {
        assert_eq!(findings.len(), 1, "expected one finding: {findings:?}");
        assert!(
            findings[0].contains(marker),
            "finding should name the {marker} marker: {findings:?}"
        );
        assert!(
            findings[0].contains("line 1"),
            "finding should name the line: {findings:?}"
        );
    }
}

#[test]
fn accepts_a_longer_run_but_rejects_a_shorter_or_spaced_one() {
    assert_eq!(conflict_marker("<<<<<<<<<"), Some('<'));
    assert_eq!(conflict_marker("======"), None);
    assert_eq!(conflict_marker("> > > > > > >"), None);
    assert_eq!(conflict_marker(""), None);
}

/// Indentation is not a marker. A merge never indents a conflict marker, and
/// an indented run is an indented code sample, so treating one as debris
/// would be a false positive on ordinary documentation.
#[test]
fn does_not_report_an_indented_marker_run() {
    let findings = reported("  =======\n    <<<<<<< HEAD\n");
    assert!(findings.is_empty(), "unexpected findings: {findings:?}");
}

/// The negative that must never regress: ordinary content that merely
/// mentions a marker. Every line here would fail a `contains` match, which
/// is the #620 defect class this rule is written not to inherit.
#[test]
fn does_not_report_marker_like_text_inside_ordinary_prose() {
    let findings = reported(
        "Use ======= to draw a section rule in this document.\n\
         The opener is <<<<<<< and the closer is >>>>>>>, as git writes them.\n\
         Inline code such as `=======` is documentation, not debris.\n\
         Compare a === b, or write x = === y in an expression.\n\
         A marker run followed by more words: ======= trailing text.\n",
    );
    assert!(findings.is_empty(), "unexpected findings: {findings:?}");
}

/// A setext heading underline shorter than the threshold is not a conflict
/// marker. This is the realistic Markdown false positive, and the threshold
/// is what keeps it out.
///
/// The honest boundary: an underline of *seven or more* is indistinguishable
/// from a conflict separator and is reported. That is a deliberate choice --
/// git's separator is exactly seven, so any longer threshold would miss the
/// defect this rule exists for. This repository uses ATX headings
/// throughout, so nothing in the tree is near the boundary; see the
/// changelog entry, which states the tradeoff rather than hiding it.
#[test]
fn does_not_report_a_setext_heading_underline_below_the_threshold() {
    for underline in ["=", "==", "===", "======"] {
        let findings = reported(&format!("Unreleased\n{underline}\n"));
        assert!(findings.is_empty(), "unexpected findings: {findings:?}");
    }
}

/// The other side of that boundary, named rather than left implicit: a
/// seven-character underline *is* reported, because it is exactly the shape
/// of a three-way merge's separator.
#[test]
fn reports_a_sized_underline_at_the_threshold_as_a_marker() {
    let findings = reported("Unreleased\n=======\n");
    assert_eq!(findings.len(), 1, "expected one finding: {findings:?}");
    assert!(findings[0].contains("line 2"), "unexpected: {findings:?}");
}

/// A marker run followed only by whitespace is still a marker, and stopping at
/// the whitespace would be a hole exactly where a rebase can leave one.
#[test]
fn reports_a_marker_run_with_trailing_whitespace() {
    assert_eq!(conflict_marker("=======   "), Some('='));
    let findings = reported("<<<<<<< HEAD   \n");
    assert_eq!(findings.len(), 1, "expected one finding: {findings:?}");
}

/// Documents the fenced-code decision: fenced blocks are **not** excluded, so
/// a marker-shaped line at the start of a line inside a fence is reported like
/// any other. Excluding fences would exempt the documentation of this very
/// defect while still letting a real artifact inside a fence through; the
/// prose in a fence is indented or mid-line and stays clean, so the common
/// case does not need the exclusion either.
#[test]
fn reports_markers_inside_a_fenced_block_rather_than_excluding_the_fence() {
    let findings = reported("```diff\n<<<<<<< HEAD\nours\n=======\n>>>>>>> feature\n```\n");
    assert_eq!(findings.len(), 3, "expected three findings: {findings:?}");
}

/// The fenced case that has to stay clean, and the reason the rule anchors to
/// the start of a line: a fence explaining the defect in prose never puts a
/// bare marker at column zero.
#[test]
fn does_not_report_prose_about_markers_inside_a_fence() {
    let findings = reported(
        "```text\nThe opener is <<<<<<< and the closer is >>>>>>>.\nA bare ======= separates them.\n```\n",
    );
    assert!(findings.is_empty(), "unexpected findings: {findings:?}");
}

/// The blank-line and end-of-file edges: an empty file is clean, and a
/// trailing newline does not hide the last line from the scan.
#[test]
fn reports_a_marker_on_the_final_line_without_a_trailing_newline() {
    let findings = reported("text\n=======");
    assert_eq!(findings.len(), 1, "expected one finding: {findings:?}");
    assert!(
        findings[0].contains("line 2"),
        "unexpected finding: {findings:?}"
    );
    assert!(reported("").is_empty(), "an empty file is clean");
}

/// Every marker form is at least [`MARKER_RUN`] long, so the constant is the
/// floor and a run of exactly the threshold must fire.
#[test]
fn fires_on_a_run_of_exactly_the_threshold_length() {
    for marker in ['<', '=', '>'] {
        let line: String = std::iter::repeat_n(marker, MARKER_RUN).collect();
        assert_eq!(conflict_marker(&line), Some(marker));
    }
}

// ------------------------------------------------------------------
// #638: the byte-level scan. Each of these goes through the real
// `findings` entry point against a file on disk whose bytes are not valid
// UTF-8. Before the fix each read failed as a whole file and the rule
// reported nothing, so every one of them fails against the old code.
// ------------------------------------------------------------------

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Result<Self, Box<dyn Error>> {
        // Artifacts stay inside the worktree's target dir: /tmp is a 512 MiB
        // tmpfs on this host and is reserved for other lanes.
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("conflict-byte-tests")
            .join(format!(
                "{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        if root.exists() {
            std::fs::remove_dir_all(&root)?;
        }
        std::fs::create_dir_all(&root)?;
        Ok(Self(root))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _cleanup = std::fs::remove_dir_all(&self.0);
    }
}

/// The issue's own reproduction. Line 2 is a complete, genuine seven-byte
/// marker at column zero; the `\xff\xfe` on line 3 is what used to hide it.
#[test]
fn reports_a_marker_in_a_file_that_is_not_valid_utf8() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let path = fixture.0.join("bad.txt");
    std::fs::write(&path, b"text\n=======\n\xff\xfe\nmore\n")?;

    let found = findings(&fixture.0, std::slice::from_ref(&path));

    assert_eq!(
        found.len(),
        1,
        "the marker on line 2 must be reported despite the invalid bytes: {found:?}"
    );
    assert_eq!(found[0].rule, "CONFLICT001");
    assert!(found[0].message.contains("line 2"), "{found:?}");
    Ok(())
}

/// A complete conflict block in a non-UTF-8 file, so the fix is not narrowly
/// limited to the bare separator: all three marker lines fire.
#[test]
fn reports_a_full_conflict_block_in_a_file_that_is_not_valid_utf8() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let path = fixture.0.join("bad.md");
    std::fs::write(
        &path,
        b"<<<<<<< HEAD\n\xff\nours\n=======\ntheirs\n>>>>>>> topic\n\xfe",
    )?;

    let found = findings(&fixture.0, std::slice::from_ref(&path));

    for line in ["line 1", "line 4", "line 6"] {
        assert!(
            found.iter().any(|finding| finding.message.contains(line)),
            "expected {line} to be reported: {found:?}"
        );
    }
    Ok(())
}

/// The counterpart that keeps the byte scan from becoming a false-positive
/// engine: a non-UTF-8 file with no marker-shaped line still reports nothing,
/// including when it mentions marker characters mid-line.
#[test]
fn reports_nothing_for_a_non_utf8_file_without_a_marker() -> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new()?;
    let path = fixture.0.join("clean.txt");
    std::fs::write(
        &path,
        b"an ordinary file\nprose about <<<<<<< and =======\n\xff\xfe",
    )?;

    let found = findings(&fixture.0, std::slice::from_ref(&path));

    assert!(found.is_empty(), "unexpected findings: {found:?}");
    Ok(())
}
