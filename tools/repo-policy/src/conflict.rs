// SPDX-License-Identifier: MIT

use std::fs;
use std::path::{Path, PathBuf};

use crate::diagnostic::Finding;
use crate::files::relative_text;

/// The three conflict-marker runs git can leave in a file, each at least seven
/// characters long. Git writes exactly seven; accepting longer runs costs nothing
/// and catches a marker padded by hand or widened by a rebase.
const MARKER_RUN: usize = 7;

/// Asserts that no tracked text file carries an unresolved conflict marker.
///
/// This is the failure mode a size gate scores as an improvement and a structure
/// gate cannot see. The stray `=======` in `CHANGELOG.md` survived a merge and
/// 14/14 green CI: it is one nonblank line, so removing it lowered the measured
/// count and passed `SIZE001`, while `DOC003` asserts the *presence* of markers,
/// not the absence of debris. `#606`'s resolution left a similar one earlier. A
/// rebase-heavy lane reintroduces exactly this, so it is checked, not trusted.
pub(crate) fn findings(root: &Path, files: &[PathBuf]) -> Vec<Finding> {
    let mut findings = Vec::new();
    for path in files {
        let relative = relative_text(root, path);
        // A file that is not UTF-8 text cannot carry a merge-conflict marker, and
        // reporting it would make this rule fail on opaque binary artifacts the
        // policy already treats as bytes. Readability is another rule to own.
        if let Ok(text) = fs::read_to_string(path) {
            check_markers(&relative, &text, &mut findings);
        }
    }
    findings
}

/// Reports every line that *begins* with a conflict-marker run.
///
/// Anchoring the run at the start of the line is load-bearing, and it is
/// deliberately not a substring match. A `contains("=======")` check would fire on
/// prose that merely mentions the marker, which is the defect #620 is fixing in
/// `DOC003`; this rule must not inherit it. A run in the middle of a sentence is
/// ordinary content.
fn check_markers(relative: &str, text: &str, findings: &mut Vec<Finding>) {
    for (index, line) in text.lines().enumerate() {
        let Some(marker) = conflict_marker(line) else {
            continue;
        };
        findings.push(Finding::error(
            "CONFLICT001",
            relative,
            format!(
                "line {} is an unresolved conflict marker ({marker}); a rebase or merge left \
                 this in a tracked file, so the file is not what it claims to be",
                index + 1
            ),
        ));
    }
}

/// Returns the marker character when `line` *starts* with a run of at least
/// [`MARKER_RUN`] of `<`, `=` or `>`.
///
/// The run need only be a prefix, because that is the shape git writes. Verified
/// against a real `git merge` conflict in this repository's default `merge`
/// conflict style, the three lines are:
///
/// ```text
/// <<<<<<< HEAD
/// =======
/// >>>>>>> side
/// ```
///
/// The opener and the closer each carry a trailing ref label; only the separator
/// is bare. Requiring the run to be the *whole* line — the first draft of this rule
/// did, and its own test caught it — would have matched the one form that reached
/// `main` while missing the two an actual rebase produces.
///
/// Anchoring at the start of the line is what keeps this from becoming a substring
/// match. A run mid-sentence is ordinary prose, and the run must reach the
/// threshold before any other character, so a short `===` in an expression is not
/// a marker. Trailing whitespace is tolerated because a rebase can leave it.
fn conflict_marker(line: &str) -> Option<char> {
    let mut characters = line.chars();
    let marker = characters.next()?;
    if !matches!(marker, '<' | '=' | '>') {
        return None;
    }
    let run_length = 1 + characters
        .by_ref()
        .take_while(|&next| next == marker)
        .count();
    // A run short of the threshold is ordinary content — a setext heading underline,
    // an `===` in an expression — rather than a conflict marker. What follows the
    // run is deliberately unconstrained: it is the ref label git appends, or
    // nothing at all for the bare separator.
    (run_length >= MARKER_RUN).then_some(marker)
}

#[cfg(test)]
mod tests {
    use super::{MARKER_RUN, check_markers, conflict_marker};
    use crate::diagnostic::Finding;

    fn reported(text: &str) -> Vec<String> {
        let mut findings = Vec::new();
        check_markers("CHANGELOG.md", text, &mut findings);
        findings.iter().map(Finding::render).collect()
    }

    #[test]
    fn identifies_each_of_the_three_marker_runs() {
        for marker in ['<', '=', '>'] {
            let line: String = std::iter::repeat_n(marker, MARKER_RUN).collect();
            assert_eq!(conflict_marker(&line), Some(marker));
        }
    }

    /// The exact state `main` shipped: a bare `=======` with no opening or closing
    /// counterpart, sitting between two changelog entries. Reporting the line number
    /// is what makes this remediable in review.
    #[test]
    fn reports_a_bare_separator_left_by_a_three_way_merge() {
        let findings = reported(
            "- **First entry.** Text.\n\
             =======\n\
             - **Second entry.** Text.\n",
        );
        assert_eq!(findings.len(), 1, "expected one finding: {findings:?}");
        assert!(
            findings[0].contains("line 2"),
            "unexpected finding: {findings:?}"
        );
    }

    /// A complete conflict block, the shape a rebase actually leaves behind. All
    /// three forms must fire: the separator alone is the case that got through.
    #[test]
    fn reports_every_line_of_a_complete_conflict_block() {
        let findings = reported(
            &[
                "<<<<<<< HEAD",
                "ours",
                "=======",
                "theirs",
                ">>>>>>> feature",
            ]
            .join("\n"),
        );
        assert_eq!(findings.len(), 3, "expected three findings: {findings:?}");
        for (position, expected) in ["line 1", "line 3", "line 5"].iter().enumerate() {
            assert!(
                findings[position].contains(expected),
                "finding {position} should name {expected}: {findings:?}"
            );
        }
    }

    #[test]
    fn accepts_a_longer_run_but_rejects_a_shorter_one() {
        assert_eq!(conflict_marker("<<<<<<<<<"), Some('<'));
        assert_eq!(conflict_marker("======"), None);
        assert_eq!(conflict_marker("> > > > > > >"), None);
    }

    /// The negative that must never regress: ordinary content that merely mentions
    /// a marker. A `contains` match would fail every one of these, which is the
    /// #620 defect class this rule is written not to inherit.
    #[test]
    fn does_not_report_marker_like_text_inside_ordinary_prose() {
        let findings = reported(
            "Use ======= to draw a section rule in this document.\n\
             The opener is <<<<<<< and the closer is >>>>>>>, as git writes them.\n\
             Inline code such as `=======` is documentation, not debris.\n\
             Compare a === b or take x = === y in an expression.\n\
             Here  <======  and  >======  appear mid-line.\n\
             A setext heading is underlined with ======.\n",
        );
        assert!(findings.is_empty(), "unexpected findings: {findings:?}");
    }

    /// A setext heading underline is a run of the marker character that is not a
    /// conflict marker, because it is shorter than the threshold. This is the
    /// realistic false positive for `=` in Markdown.
    #[test]
    fn does_not_report_a_setext_heading_underline() {
        let findings = reported("Unreleased\n======\n");
        assert!(findings.is_empty(), "unexpected findings: {findings:?}");
    }

    /// A run may be followed by trailing whitespace or by the ref label git
    /// appends; neither is a reason to miss the marker.
    #[test]
    fn reports_a_marker_run_with_trailing_whitespace() {
        assert_eq!(conflict_marker("=======  \t"), Some('='));
        let findings = reported("<<<<<<<   \n");
        assert_eq!(findings.len(), 1, "expected one finding: {findings:?}");
    }
}
