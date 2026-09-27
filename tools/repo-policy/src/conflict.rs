// SPDX-License-Identifier: MIT

use std::fs;
use std::path::{Path, PathBuf};

use crate::diagnostic::Finding;
use crate::files::relative_text;

/// The number of repeated characters git writes for a conflict marker, and the
/// threshold this rule treats as one. Git always writes exactly seven; a longer
/// run is a marker someone padded by hand or a rebase widened, and costs nothing
/// to catch.
const MARKER_RUN: usize = 7;

/// Reports every tracked file that still carries an unresolved conflict marker.
///
/// The stray `=======` that sat in `CHANGELOG.md` survived a merge and fourteen
/// green CI runs, and it survived because both gates covering that file are
/// shaped so that it passes: the size rule scored the removal as a *reduction*
/// in nonblank lines, and `DOC003` asserts that markers are *present*, not that
/// conflict debris is absent. Each instance is cited by the commit that
/// introduced it, and each removal by the commit that removed it. Every
/// citation below is a commit, never a branch tip, because a tip can be closed
/// or superseded while the prose still names it, and `main` is the only ref
/// whose state is asserted. `501a711` (the #601 merge) introduced the line-29
/// artifact, reached `main`, and #618 removed it; `8e3ffea` added a second at
/// line 45, which no commit in its own history has removed and which is not on
/// `main`; `696e56f` added one at line 60, which `cba9be8` removed; `df9ef25`
/// added one at line 100, which `94802b8` removed. Of the four, only `501a711`
/// is on `main`.
/// A rebase across the changelog boundary reintroduces exactly this, so it is
/// checked rather than trusted.
///
/// Fenced code blocks are deliberately **not** excluded. A conflict marker inside
/// a fence is a line that is entirely a marker run, and the only way this repo
/// can document one is to show it in a fenced block -- which is the shape this
/// rule's own negative test uses. Excluding fences would therefore exempt the
/// documentation of the defect while still catching a real rebase artifact that
/// happens to land inside a fence, and it would require a fence-tracking parser
/// with its own untested edge cases (unclosed fences, fences inside fences, ````
/// -in-different-markdown-dialects). The whole-line test is narrow enough that
/// the false-positive surface is empty: see the setext-underline test.
pub(crate) fn findings(root: &Path, files: &[PathBuf]) -> Vec<Finding> {
    let mut findings = Vec::new();
    for path in files {
        let relative = relative_text(root, path);
        // A file that is not UTF-8 cannot carry a marker: the bytes of `<`, `=` and
        // `>` are not part of any multi-byte sequence, so a run of them in a
        // non-UTF-8 file cannot be one either. Reading text is therefore the
        // right filter, and it leaves binary artifacts to the rules that own them.
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        check_markers(&relative, &text, &mut findings);
    }
    findings
}

/// Reports every line that *starts* with a run of at least [`MARKER_RUN`] of one
/// marker character.
///
/// The positional test is load-bearing, and it is deliberately not a substring
/// match. `text.contains("=======")` would fire on prose that merely names a
/// marker anywhere in a line -- the defect #620 is fixing in `DOC003` -- and this
/// rule must not inherit it. An equals run inside a sentence is ordinary content.
///
/// Matching from the start of the line rather than requiring the line to be
/// *entirely* a run is deliberate, and it is the difference between a gate that
/// works and one that does not. Git writes its opener and closer with a label
/// attached:
///
/// ```text
/// <<<<<<< HEAD
/// ours
/// =======
/// theirs
/// >>>>>>> topic
/// ```
///
/// A whole-line test matches only the bare `=======` separator and misses the
/// two lines that actually bracket a real conflict -- so it would catch the
/// orphaned artifact in `CHANGELOG.md` and then stay silent through every future
/// rebase that leaves a complete block behind. Everything after the run is
/// unconstrained, because that is where git puts the branch and label names.
fn check_markers(relative: &str, text: &str, findings: &mut Vec<Finding>) {
    for (index, line) in text.lines().enumerate() {
        let Some(marker) = conflict_marker(line) else {
            continue;
        };
        findings.push(Finding::error(
            "CONFLICT001",
            relative,
            format!(
                "line {} is an unresolved conflict marker ({marker}): a rebase or merge left \
                 this in a tracked file, so the file is not the content it claims to be",
                index + 1
            ),
        ));
    }
}

/// Returns the marker character when `line` begins with a run of at least
/// [`MARKER_RUN`] of `<`, `=` or `>`.
fn conflict_marker(line: &str) -> Option<char> {
    let marker = line.chars().next()?;
    if !matches!(marker, '<' | '=' | '>') {
        return None;
    }
    // Leading indentation is never produced by a merge, and tolerating it would
    // widen the rule into matching an indented code sample; a marker in a real
    // artifact always begins at column zero.
    let run = 1 + line
        .chars()
        .skip(1)
        .take_while(|&next| next == marker)
        .count();
    (run >= MARKER_RUN).then_some(marker)
}

#[cfg(test)]
mod tests {
    use super::{MARKER_RUN, check_markers, conflict_marker};

    fn reported(text: &str) -> Vec<String> {
        let mut findings = Vec::new();
        check_markers("CHANGELOG.md", text, &mut findings);
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
}
