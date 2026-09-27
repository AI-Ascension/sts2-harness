// SPDX-License-Identifier: MIT

use std::path::{Path, PathBuf};

use crate::byte_scan::read_bytes;
use crate::diagnostic::Finding;
use crate::files::relative_text;

/// The number of repeated characters git writes for a conflict marker, and the
/// threshold this rule treats as one. Git always writes exactly seven; a longer
/// run is a marker someone padded by hand or a rebase widened, and costs nothing
/// to catch.
const MARKER_RUN: usize = 7;

/// Reports every tracked file that still carries an unresolved conflict marker.
///
/// The stray `=======` that sat in `CHANGELOG.md` survived a merge and a full
/// round of green CI, and it survived because both gates covering that file are
/// shaped so that it passes: the size rule scored the removal as a *reduction*
/// in nonblank lines, and `DOC003` asserts that markers are *present*, not that
/// conflict debris is absent. Each instance is cited by the commit that
/// introduced it, and each removal by the commit that removed it. No clause
/// rests on a branch, a rebase, or a count, and every citation is a commit
/// rather than a branch tip, because a tip can be closed or superseded while
/// the prose still names it. `main` is the only ref whose state is asserted.
/// `501a711` (the #601 merge) introduced the line-29 artifact, reached `main`,
/// and `06eba7e6` (#618) removed it; `8e3ffea` added a second at line 45,
/// which no commit in its own history has removed and which is not on `main`;
/// `696e56f` added one at line 60, which `cba9be8` removed; `df9ef25` added one
/// at line 100, which `94802b8` removed. Of the four, only `501a711` is on
/// `main`.
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
        // Scan bytes, never decoded text. `<`, `=` and `>` are ASCII, so a marker
        // run cannot be *split* across a multi-byte sequence -- that much is true.
        // But the inference this rule used to draw from it was wrong: a decode
        // failure is not per line. One non-UTF-8 byte anywhere fails the read of
        // the entire file, so reading text to filter out "binary" files silently
        // exempted a whole file from this rule, and from every other rule that
        // read it the same way, on the strength of a single stray byte. Matching
        // raw bytes has no decode step to fail, so no file can be skipped here.
        let Some(bytes) = read_bytes(path) else {
            continue;
        };
        check_markers(&relative, &bytes, &mut findings);
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
fn check_markers(relative: &str, bytes: &[u8], findings: &mut Vec<Finding>) {
    for (index, line) in bytes.split(|byte| *byte == b'\n').enumerate() {
        let Some(marker) = line_marker(line) else {
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

/// Returns the marker byte when `line` begins with a run of at least
/// [`MARKER_RUN`] of `<`, `=` or `>`.
fn line_marker(line: &[u8]) -> Option<char> {
    let marker = *line.first()?;
    if !matches!(marker, b'<' | b'=' | b'>') {
        return None;
    }
    // Leading indentation is never produced by a merge, and tolerating it would
    // widen the rule into matching an indented code sample; a marker in a real
    // artifact always begins at column zero.
    let run = line.iter().take_while(|byte| **byte == marker).count();
    (run >= MARKER_RUN).then_some(marker as char)
}

#[cfg(test)]
#[path = "conflict_tests.rs"]
mod tests;

/// Returns the marker character when `line` begins with a run of at least
/// [`MARKER_RUN`] of `<`, `=` or `>`.
///
/// The decoded-string form of the test, kept for the boundary and negative
/// cases that read more clearly as strings. Production scanning is
/// [`line_marker`], which works on bytes so it cannot be skipped for an
/// unrelated invalid byte elsewhere in the file.
#[cfg(test)]
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
