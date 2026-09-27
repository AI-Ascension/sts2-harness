// SPDX-License-Identifier: MIT

use std::fs;
use std::path::{Path, PathBuf};

use crate::config::Policy;
use crate::diagnostic::Finding;
use crate::files::relative_text;

pub(crate) fn findings(root: &Path, files: &[PathBuf], policy: &Policy) -> Vec<Finding> {
    let mut findings = Vec::new();
    for path in files
        .iter()
        .filter(|path| path.extension().is_some_and(|value| value == "md"))
    {
        let relative = relative_text(root, path);
        match fs::read_to_string(path) {
            Ok(text) => {
                check_links(path, &relative, &text, &mut findings);
                check_required_preamble(&relative, &text, policy, &mut findings);
            }
            Err(error) => findings.push(Finding::error(
                "DOC002",
                &relative,
                format!("cannot read Markdown: {error}"),
            )),
        }
    }
    findings
}

/// Asserts that a file whose identity the policy depends on still opens with
/// its declared markers, in order.
///
/// A size gate cannot cover this: every line of a changelog's preamble is
/// nonblank, so deleting the preamble entirely *reduces* the measured count
/// and passes. The repository lost `CHANGELOG.md`'s title, preamble and
/// `## Unreleased` heading exactly that way, with `repo-policy --strict`
/// green throughout.
///
/// A marker matches a **whole line**, not a substring, and the first marker
/// must be the file's first nonblank line. Substring matching made the rule
/// inert for exactly the marker it existed to protect: `CHANGELOG.md`
/// describes its own `## Unreleased` heading in prose, so deleting the
/// heading while the prose survived still satisfied `text.contains`. Whole
/// lines close that, and anchoring the first marker is what makes "opens
/// with" mean something — a file whose title has been pushed down or
/// replaced has lost its identity even if every marker still appears.
fn check_required_preamble(
    relative: &str,
    text: &str,
    policy: &Policy,
    findings: &mut Vec<Finding>,
) {
    let Some(markers) = policy.required_preambles.get(relative) else {
        return;
    };
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    let opening = lines.iter().find(|line| !line.trim().is_empty()).copied();

    // Ordered scan rather than a per-marker search. `contains` never checked
    // order despite the doc comment claiming it, so a file carrying its
    // markers in reverse passed; and a search per marker would let a single
    // line satisfy several markers, so a file repeating one heading could
    // stand in for the whole preamble. This walks the file once and lets each
    // line consume at most the next unclaimed marker.
    let mut missing = vec![true; markers.len()];
    let mut cursor = 0;
    for line in &lines {
        if cursor >= markers.len() {
            break;
        }
        if line.trim() == markers[cursor].trim() {
            missing[cursor] = false;
            cursor += 1;
        }
    }

    // The first marker is not just present, it *opens* the file. A title that
    // survives further down has still lost the identity this rule asserts, so
    // the ordered scan's "found it somewhere" result does not clear it.
    if let Some(first) = markers.first()
        && opening != Some(first.trim())
    {
        missing[0] = true;
    }

    for (index, marker) in markers.iter().enumerate() {
        if !missing[index] {
            continue;
        }
        let detail = match (index, opening) {
            (0, Some(opening)) if opening != marker.trim() => {
                format!(
                    "required structural marker {index} is not the file's opening line: \
                     {marker} (found {opening:?} instead);"
                )
            }
            _ => format!("required structural marker {index} is missing: {marker};"),
        };
        findings.push(Finding::error(
            "DOC003",
            relative,
            format!(
                "{detail} this file's identity is asserted by policy.toml and is not \
                 covered by any size check"
            ),
        ));
    }
}

fn check_links(path: &Path, relative: &str, text: &str, findings: &mut Vec<Finding>) {
    for target in link_targets(text) {
        let target = target.trim().trim_matches(['<', '>']);
        let local = target.split('#').next().unwrap_or("");
        if local.is_empty()
            || local.starts_with("http://")
            || local.starts_with("https://")
            || local.starts_with("mailto:")
        {
            continue;
        }
        let resolved = path.parent().unwrap_or_else(|| Path::new("")).join(local);
        if !resolved.exists() {
            findings.push(Finding::error(
                "DOC002",
                relative,
                format!("local Markdown link target does not exist: {local}"),
            ));
        }
    }
}

fn link_targets(text: &str) -> impl Iterator<Item = &str> {
    text.match_indices("](").filter_map(|(start, _)| {
        let target = &text[start + 2..];
        let end = target.find(')')?;
        Some(&target[..end])
    })
}

#[cfg(test)]
#[path = "markdown_tests.rs"]
mod tests;
