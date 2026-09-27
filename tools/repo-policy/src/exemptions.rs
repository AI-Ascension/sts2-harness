// SPDX-License-Identifier: MIT

//! `EXC001`–`EXC004`: an exemption waives a size limit, so its prose is the only record of why.
//!
//! `size_findings` skips an exempted file outright, so the sentence in `policy.toml` is the only
//! place a reader learns whether the file is still inside its budget. That sentence embeds the
//! nonblank line count, and nothing ever checked it against the file, so a stale count could hide
//! a real breach while every gate stayed green.
//!
//! Three separate failures are now reported, because they call for different responses:
//!
//! * `EXC002` — the stated count is wrong but the file is still within its hard maximum. The
//!   sentence is simply stale.
//! * `EXC003` — the file is over its hard maximum. Either the count is stale as well, or the
//!   count is right and the exemption is waiving a breach nobody can see. Both need a tracked
//!   owner, so both report the same rule with a message naming which case it is.
//! * `EXC004` — the prose cites a hard limit that disagrees with `[limits]`. A limit is only
//!   ever widened in `[limits]`, so a contradicting sentence is a stale copy of a budget.
//!
//! A file over its hard maximum is only accepted when the reason carries the
//! `over-hard-maximum: #NNN` marker naming the tracked issue that owns the breach. That makes the
//! waiver machine-checkable and greppable: the exemption can no longer absorb a breach into
//! silence, and the debt always points at the issue that has to be closed by splitting the file.

use std::fs;
use std::path::{Component, Path};

use crate::config::Policy;
use crate::diagnostic::Finding;
use crate::files::size_category;

/// The clause a reason must carry to waive a hard-maximum breach, naming the tracked issue.
const WAIVER_MARKER: &str = "over-hard-maximum:";
/// How many words may sit between a cited limit and the `hard limit` phrase that names it.
const BUDGET_WINDOW: usize = 3;

pub(crate) fn findings(root: &Path, policy: &Policy) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (relative, reason) in &policy.exemptions {
        let path = Path::new(relative);
        if path.is_absolute() || path.components().any(|part| part == Component::ParentDir) {
            findings.push(Finding::error(
                "EXC001",
                relative,
                "exemption must be an exact repository-relative path",
            ));
            continue;
        }
        if reason.trim().len() < 20 {
            findings.push(Finding::error(
                "EXC001",
                relative,
                "exemption reason must contain at least 20 characters",
            ));
            continue;
        }
        let full = root.join(path);
        if !full.is_file() {
            findings.push(Finding::error(
                "EXC001",
                relative,
                "exempted file does not exist",
            ));
            continue;
        }
        findings.extend(count_findings(relative, &full, reason, policy));
    }
    findings
}

/// Compares the numbers the reason states against the file and against `[limits]`.
fn count_findings(relative: &str, path: &Path, reason: &str, policy: &Policy) -> Vec<Finding> {
    // Fail closed: a reason whose counts cannot be told apart is itself the defect, because there
    // is no way to know which number a reviewer was meant to check.
    let Some(Claimed { size, maximum }) = split_claims(reason) else {
        return vec![Finding::error(
            "EXC002",
            relative,
            "exemption reason states more than one line count; keep exactly one, so the file's \
             size can be verified against it",
        )];
    };
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => {
            return vec![Finding::error(
                "EXC001",
                relative,
                format!("cannot read UTF-8 text: {error}"),
            )];
        }
    };
    let actual = nonblank(&text);
    let budget = hard_maximum(path, policy);
    let over = budget.is_some_and(|budget| actual > budget);
    let mut findings = Vec::new();

    if let Some(claimed) = size.filter(|claimed| *claimed != actual) {
        findings.push(Finding::error(
            if over { "EXC003" } else { "EXC002" },
            relative,
            mismatch_message(claimed, actual, over, budget),
        ));
    }
    if over && waiver(reason).is_none() {
        findings.push(Finding::error(
            "EXC003",
            relative,
            format!(
                "exemption waives a real breach: {actual} nonblank lines exceed the hard maximum \
                 of {maximum}; add `{WAIVER_MARKER} #NNN` naming the tracked issue that owns it",
                maximum = budget.unwrap_or_default()
            ),
        ));
    }
    if let Some((claimed, budget)) = maximum
        .zip(budget)
        .filter(|(claimed, budget)| claimed != budget)
    {
        findings.push(Finding::error(
            "EXC004",
            relative,
            format!(
                "exemption reason cites a {claimed}-line hard limit but [limits] sets \
                 {budget}; a budget is widened in [limits], never in prose"
            ),
        ));
    }
    findings
}

fn mismatch_message(claimed: usize, actual: usize, over: bool, budget: Option<usize>) -> String {
    let tail = match budget {
        Some(budget) if over => {
            format!(", and the exemption is waiving that breach against a hard maximum of {budget}")
        }
        Some(budget) => format!(", which stays within its hard maximum of {budget}"),
        None => String::new(),
    };
    format!("exemption reason states {claimed} nonblank lines but the file has {actual}{tail}")
}

/// The tracked issue a reason names as the owner of a waived breach, if it names one properly.
fn waiver(reason: &str) -> Option<u64> {
    let (_, tail) = reason.split_once(WAIVER_MARKER)?;
    let number: String = tail
        .trim_start()
        .chars()
        .skip_while(|character| *character == '#')
        .take_while(char::is_ascii_digit)
        .collect();
    (!number.is_empty()).then(|| number.parse().ok())?
}

/// The two numbers a reason can state: the file's own size, and the budget it is excused from.
#[derive(Debug, Eq, PartialEq)]
struct Claimed {
    size: Option<usize>,
    maximum: Option<usize>,
}

/// Splits the prose into its size claim and its cited budget, failing closed when ambiguous.
fn split_claims(reason: &str) -> Option<Claimed> {
    let tokens: Vec<&str> = reason.split_whitespace().collect();
    let mut size: Option<usize> = None;
    let mut maximum: Option<usize> = None;
    let mut index = 0;
    while index < tokens.len() {
        let hyphenated = hyphenated_count(tokens[index]);
        let Some(claimed) = hyphenated.or_else(|| spaced_count(&tokens, index)) else {
            index += 1;
            continue;
        };
        let width = if hyphenated.is_some() {
            1
        } else {
            line_claim_width(&tokens, index)
        };
        // A number only names the budget when `hard limit` sits close behind it, as in
        // `the 700-line markdown hard limit`. Further away, the phrase belongs to the
        // sentence's own claim, as in `its 334-line implementation is below the hard limit`.
        if names_hard_limit(&tokens[index + width..]) {
            if maximum.replace(claimed).is_some() {
                return None;
            }
        } else if size.replace(claimed).is_some() {
            return None;
        }
        index += width;
    }
    Some(Claimed { size, maximum })
}

/// True when `hard limit` names the number just parsed, rather than a later clause.
fn names_hard_limit(tail: &[&str]) -> bool {
    let window = tail
        .iter()
        .take(BUDGET_WINDOW)
        .copied()
        .collect::<Vec<_>>()
        .join(" ");
    window.contains("hard limit") || window.contains("hard maximum")
}

/// The value of a `<number> [nonblank] line(s)` claim spread across separate tokens.
fn spaced_count(tokens: &[&str], index: usize) -> Option<usize> {
    let rest = &tokens[index + 1..];
    let value = leading_digits(tokens[index])?;
    let names_lines = is_line_word(rest.first().copied().unwrap_or_default())
        || (rest.first() == Some(&"nonblank")
            && is_line_word(rest.get(1).copied().unwrap_or_default()));
    names_lines.then_some(value)
}

/// `322-line`: the value in a `number-dash-unit` token, when the unit counts lines.
fn hyphenated_count(token: &str) -> Option<usize> {
    let (digits, unit) = token.split_once([
        '\u{2010}', '\u{2011}', '\u{2012}', '\u{2013}', '\u{2014}', '\u{2015}', '-',
    ])?;
    is_line_word(unit).then(|| parse_usize(digits))?
}

/// How many whitespace tokens one `<number> [nonblank] line(s)` claim occupies.
fn line_claim_width(tokens: &[&str], index: usize) -> usize {
    if tokens.get(index + 1) == Some(&"nonblank") {
        3
    } else {
        2
    }
}

fn is_line_word(token: &str) -> bool {
    matches!(
        token.trim_end_matches(['.', ',', ';', ':', ')', '"', '\'']),
        "line" | "lines"
    )
}

fn leading_digits(token: &str) -> Option<usize> {
    let digits: String = token.chars().take_while(char::is_ascii_digit).collect();
    parse_usize(&digits)
}

fn parse_usize(digits: &str) -> Option<usize> {
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.bytes().try_fold(0usize, |value, byte| {
        value.checked_mul(10)?.checked_add(usize::from(byte - b'0'))
    })
}

/// The hard maximum that applies to `path`, if the file has a size category at all.
fn hard_maximum(path: &Path, policy: &Policy) -> Option<usize> {
    let category = size_category(path)?;
    policy.budget(category).map(|budget| budget.maximum)
}

fn nonblank(text: &str) -> usize {
    text.lines().filter(|line| !line.trim().is_empty()).count()
}

#[cfg(test)]
#[path = "exemptions_tests.rs"]
mod exemptions_tests;
