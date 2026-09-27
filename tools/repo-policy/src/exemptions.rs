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
/// count, which is the artifact that makes a naive scan report 23 stale entries instead
/// of 21. Fails closed: a sentence that clearly asserts a count but does not match the
/// grammar yields `Some(0)`, never `None`, so an unparseable claim is reported rather
/// than trusted.
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
            None => match tail.strip_prefix("-line") {
                Some(rest) => rest.is_empty() || rest.starts_with(' '),
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
    // Fail closed on an asserted-but-unparseable count.
    if reason.contains("nonblank lines")
        || reason.contains(" nonblank line")
        || reason.contains("-line")
    {
        return Some(0);
    }
    None
}

#[cfg(test)]
#[path = "exemption_tests.rs"]
mod tests;
