// SPDX-License-Identifier: MIT

//! Exemption evidence: the stated line count in an exemption reason is the only surviving
//! record that an exempted file is over budget, because the size check skips exempted files
//! entirely. These helpers verify that record instead of trusting it.

use std::fs;
use std::path::Path;

use crate::config::Policy;
use crate::diagnostic::Finding;

use crate::files::size_category;

/// The stated line count in an exemption reason is the only surviving record that an
/// exempted file is over budget, because `size_findings` skips the check entirely. An
/// exemption therefore waives the limit but never the evidence: a count that no longer
/// describes the file is reported here rather than silently trusted.
pub(crate) fn exemption_count_findings(
    root: &Path,
    relative: &str,
    reason: &str,
    policy: &Policy,
) -> Vec<Finding> {
    let Some(claimed) = stated_line_count(reason) else {
        return Vec::new();
    };
    let path = root.join(relative);
    let Ok(text) = fs::read_to_string(&path) else {
        return Vec::new();
    };
    let actual = text.lines().filter(|line| !line.trim().is_empty()).count();
    if claimed == actual {
        return Vec::new();
    }
    if claimed == 0 {
        return vec![Finding::error(
            "EXC002",
            relative,
            format!(
                "exemption asserts a nonblank line count that does not parse, but the file has \
                 {actual}; an unverified count is never treated as a pass"
            ),
        )];
    }
    let Some(category) = size_category(path.strip_prefix(root).unwrap_or(&path)) else {
        return vec![Finding::error(
            "EXC002",
            relative,
            format!(
                "exemption states {claimed} nonblank lines but the file has {actual}, and its \
                 size category has no configured budget to compare against"
            ),
        )];
    };
    let over_hard_limit = policy
        .budget(category)
        .is_some_and(|budget| actual > budget.maximum);
    // A stale count on a file that is genuinely over its hard maximum is not prose drift:
    // the exemption is hiding a real breach behind a number that understates it, so it is
    // an error. Stale prose on a compliant file stays a warning.
    let finding = if over_hard_limit {
        Finding::error(
            "EXC002",
            relative,
            format!(
                "exemption states {claimed} nonblank lines but the file has {actual}, which is \
                 over the hard maximum; the exemption waives the limit but does not excuse the \
                 breach"
            ),
        )
    } else {
        Finding::warning(
            "EXC002",
            relative,
            format!("exemption states {claimed} nonblank lines but the file has {actual}"),
        )
    };
    vec![finding]
}

/// Extracts the file's own stated line count from an exemption reason.
///
/// The table's grammar binds the count to the subject with `its` (`its 632 nonblank
/// lines`), while the applicable limit is introduced by `the` (`the 700-line markdown
/// hard limit`). Matching only the subject form keeps a limit from being read as the
/// count. Three entries in the real table state **both** numbers, so a scan that takes
/// whichever number it sees first would read the limit and report a file that is
/// perfectly compliant as stale. Fails closed: a sentence that clearly asserts a count
/// but does not match the grammar yields `Some(0)`, never `None`, so an unparseable claim
/// is reported rather than trusted.
pub(crate) fn stated_line_count(reason: &str) -> Option<usize> {
    const SUBJECT_COUNT: &str = "its ";
    let mut found = None;
    let mut rest = reason;
    while let Some(index) = rest.find(SUBJECT_COUNT) {
        let after = &rest[index + SUBJECT_COUNT.len()..];
        let Some(end) = after.find(|character: char| !character.is_ascii_digit()) else {
            break;
        };
        let digits = &after[..end];
        let tail = after[end..].trim_start();
        let is_count_phrase = match tail.strip_prefix("nonblank ") {
            Some(rest) => rest.starts_with("line"),
            // `its <N>-line` is only a count when a noun describing the file follows, as in
            // `its 334-line implementation`. Without that noun the phrase is
            // `... below its 400-line hard limit`, where 400 is the *limit*, not the file's
            // size — reading it as a count would fail a perfectly compliant file. The
            // `-line` form has no `the` asymmetry to protect it the way `nonblank lines`
            // does, so the noun requirement is what keeps the two apart. A noun that names a
            // *limit* disqualifies the phrase for the same reason.
            None => match tail.strip_prefix("-line") {
                Some(rest) => {
                    let noun = rest
                        .trim_start()
                        .split(|character: char| !character.is_ascii_alphabetic())
                        .find(|word| !word.is_empty())
                        .unwrap_or_default()
                        .to_ascii_lowercase();
                    !noun.is_empty()
                        && !matches!(
                            noun.as_str(),
                            "hard" | "limit" | "maximum" | "max" | "budget" | "cap" | "ceiling"
                        )
                }
                None => false,
            },
        };
        if is_count_phrase {
            found = digits.parse::<usize>().ok();
        }
        if found.is_some() {
            break;
        }
        rest = after;
    }
    if found.is_some() {
        return found;
    }
    // Fail closed on an asserted-but-unparseable count. The net is deliberately narrow: it
    // must catch a sentence that claims a line count and cannot be read, without firing on
    // a phrase that merely restates a *limit*. `its 400-line hard limit` states a limit and
    // claims nothing about the file, so treating it as an unparseable count would fail a
    // compliant file for a benign rewording — the same false positive the limit-vs-count
    // distinction exists to prevent.
    if asserts_a_count(reason) {
        return Some(0);
    }
    None
}

/// Whether a reason asserts a line count about the exempted file, as opposed to restating a
/// limit. Used only to decide whether a failed parse should be reported.
fn asserts_a_count(reason: &str) -> bool {
    let lowered = reason.to_ascii_lowercase();
    // "its N nonblank lines" in any wording, e.g. "it is 500 nonblank lines".
    if lowered.contains("nonblank line") {
        return true;
    }
    // "its N-line <noun>", excluding limit nouns.
    lowered.match_indices("-line").any(|(index, _)| {
        let rest = lowered[index + "-line".len()..].trim_start();
        let noun = rest
            .split(|character: char| !character.is_ascii_alphabetic())
            .find(|word| !word.is_empty())
            .unwrap_or_default();
        !noun.is_empty()
            && !matches!(
                noun,
                "hard" | "limit" | "maximum" | "max" | "budget" | "cap" | "ceiling"
            )
    })
}

#[cfg(test)]
#[path = "exemption_tests.rs"]
mod tests;
