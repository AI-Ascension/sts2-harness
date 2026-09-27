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
fn check_required_preamble(
    relative: &str,
    text: &str,
    policy: &Policy,
    findings: &mut Vec<Finding>,
) {
    let Some(markers) = policy.required_preambles.get(relative) else {
        return;
    };
    for (index, marker) in markers.iter().enumerate() {
        if !text.contains(marker) {
            findings.push(Finding::error(
                "DOC003",
                relative,
                format!(
                    "required structural marker {index} is missing: {marker}; \
                     this file's identity is asserted by policy.toml and is not \
                     covered by any size check"
                ),
            ));
        }
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
mod tests {
    use super::{check_required_preamble, link_targets};
    use crate::config::Policy;
    use std::collections::BTreeMap;

    #[test]
    fn extracts_inline_markdown_targets() {
        let targets: Vec<_> =
            link_targets("[one](docs/one.md) and [two](https://example.test)").collect();
        assert_eq!(targets, ["docs/one.md", "https://example.test"]);
    }

    fn policy_with_changelog_markers() -> Policy {
        let mut required_preambles = BTreeMap::new();
        required_preambles.insert(
            "CHANGELOG.md".to_owned(),
            vec!["# Changelog".to_owned(), "## Unreleased".to_owned()],
        );
        Policy::with_preambles(required_preambles)
    }

    #[test]
    fn a_changelog_carrying_its_markers_produces_no_finding() {
        let policy = policy_with_changelog_markers();
        let mut findings = Vec::new();
        check_required_preamble(
            "CHANGELOG.md",
            "# Changelog\n\n## Unreleased\n\n- an entry\n",
            &policy,
            &mut findings,
        );
        assert!(findings.is_empty(), "unexpected findings: {findings:?}");
    }

    /// The exact state `main` was left in by the #606 merge: the bullet list
    /// promoted to the top of the file, every preamble line deleted. This is
    /// the regression the rule exists to catch, and the size gate passes it.
    #[test]
    fn a_changelog_that_lost_its_title_and_unreleased_heading_is_reported() {
        let policy = policy_with_changelog_markers();
        let mut findings = Vec::new();
        check_required_preamble(
            "CHANGELOG.md",
            "- **An entry.** With the preamble gone.\n",
            &policy,
            &mut findings,
        );
        assert_eq!(
            findings.len(),
            2,
            "expected both markers reported: {findings:?}"
        );
        assert!(findings.iter().all(|finding| finding.rule == "DOC003"));
    }

    #[test]
    fn a_file_outside_the_preamble_table_is_not_checked() {
        let policy = policy_with_changelog_markers();
        let mut findings = Vec::new();
        check_required_preamble("README.md", "no markers here\n", &policy, &mut findings);
        assert!(findings.is_empty(), "unexpected findings: {findings:?}");
    }
}
