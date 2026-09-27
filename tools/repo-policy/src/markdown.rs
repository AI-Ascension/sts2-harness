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
/// The markers are matched as whole lines rather than as substrings, and the
/// first of them must be the file's first nonblank line. A substring match
/// makes a marker satisfiable by prose: `CHANGELOG.md` describes its own lost
/// `## Unreleased` heading in an entry, so on a tree where the real heading
/// was deleted the marker was still "present" and the rule was inert for it.
/// Anchoring the first marker is what "preamble" means — a file whose title
/// has been pushed down or replaced by prose has lost the structure the policy
/// depends on even if both marker lines still exist somewhere further in.
fn check_required_preamble(
    relative: &str,
    text: &str,
    policy: &Policy,
    findings: &mut Vec<Finding>,
) {
    let Some(markers) = policy.required_preambles.get(relative) else {
        return;
    };
    let mut lines = text.lines().map(str::trim_end);
    let first_nonblank = lines.clone().find(|line| !line.is_empty());
    // A marker is only satisfied by a line that is exactly it, appearing after every
    // marker declared before it. Both halves matter: the equality rejects a prose
    // mention, and the order rejects a file that still carries the lines but no longer
    // opens with them in the declared sequence. One pass finds the longest prefix of the
    // declared markers that appears as whole lines in order; a marker past that prefix is
    // the first one out of place, and it is where the file stops being a preamble.
    let mut satisfied = 0;
    for line in lines.by_ref() {
        if markers.get(satisfied).map(String::as_str) == Some(line) {
            satisfied += 1;
        }
    }
    for (index, marker) in markers.iter().enumerate().skip(satisfied) {
        findings.push(Finding::error(
            "DOC003",
            relative,
            format!(
                "required structural marker {index} is missing: {marker}; \
                 this file's identity is asserted by policy.toml and is not \
                 covered by any size check; markers must appear as whole lines \
                 in the declared order, and the first must be the first nonblank line"
            ),
        ));
    }
    if let Some(first) = markers.first() {
        if first_nonblank != Some(first.as_str()) {
            findings.push(Finding::error(
                "DOC003",
                relative,
                format!(
                    "the first nonblank line must be the first declared structural \
                     marker {first}; found {found:?}; the preamble is what identifies \
                     this file, and it is not covered by any size check",
                    found = first_nonblank.unwrap_or("<blank file>")
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
            3,
            "expected both markers reported plus the displaced anchor: {findings:?}"
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

    /// The half of the preamble a substring match silently cannot see.
    ///
    /// `CHANGELOG.md` quotes the literal string `## Unreleased` in the entry that
    /// describes losing the heading, so deleting the real heading left a text that
    /// still `contains` the marker. This is that exact state, and it must fail.
    #[test]
    fn a_prose_mention_does_not_satisfy_a_marker() {
        let policy = policy_with_changelog_markers();
        let mut findings = Vec::new();
        check_required_preamble(
            "CHANGELOG.md",
            "# Changelog\n\nAll notable changes are documented here.\n\n\
             The `#606` merge dropped the `## Unreleased` heading and every gate \
             stayed green.\n",
            &policy,
            &mut findings,
        );
        assert_eq!(
            findings.len(),
            1,
            "the prose mention must not satisfy the missing heading: {findings:?}"
        );
        assert!(
            findings[0].message.contains("## Unreleased"),
            "the reported marker must be the one only prose mentions: {findings:?}"
        );
    }

    /// A marker is a whole line, not a prefix: a heading with trailing content
    /// appended to the same line is not the declared structure.
    #[test]
    fn a_marker_with_trailing_content_on_its_line_is_rejected() {
        let policy = policy_with_changelog_markers();
        let mut findings = Vec::new();
        check_required_preamble(
            "CHANGELOG.md",
            "# Changelog of the project\n\n## Unreleased\n\n- an entry\n",
            &policy,
            &mut findings,
        );
        assert_eq!(
            findings.len(),
            3,
            "the unmatched title, the unreached heading and the wrong first line must all \
             be reported: {findings:?}"
        );
    }

    /// "Preamble" means the markers open the file. A title that survives but has
    /// been pushed below other content no longer identifies the file, and the
    /// substring check passed it for as long as the line was anywhere at all.
    #[test]
    fn a_title_pushed_below_other_content_is_reported() {
        let policy = policy_with_changelog_markers();
        let mut findings = Vec::new();
        check_required_preamble(
            "CHANGELOG.md",
            "All notable changes are documented here.\n\n# Changelog\n\n## Unreleased\n\n- an entry\n",
            &policy,
            &mut findings,
        );
        assert_eq!(
            findings.len(),
            1,
            "both markers are present, so only the anchor may fail: {findings:?}"
        );
        assert!(
            findings[0].message.contains("first nonblank line"),
            "the anchor failure must name the cause: {findings:?}"
        );
    }

    /// Both marker lines survive, in the wrong order. The rule's own doc comment
    /// claimed markers were checked in order; nothing enforced it.
    #[test]
    fn markers_present_in_the_wrong_order_are_reported() {
        let policy = policy_with_changelog_markers();
        let mut findings = Vec::new();
        check_required_preamble(
            "CHANGELOG.md",
            "## Unreleased\n\n- an entry\n\n# Changelog\n",
            &policy,
            &mut findings,
        );
        assert!(
            !findings.is_empty(),
            "markers out of declared order must be reported"
        );
        assert!(findings.iter().all(|finding| finding.rule == "DOC003"));
    }

    /// Blank lines ahead of the preamble do not displace it: the contract is the
    /// first *nonblank* line, so a leading blank run is tolerated on purpose.
    #[test]
    fn a_leading_blank_line_does_not_displace_the_preamble() {
        let policy = policy_with_changelog_markers();
        let mut findings = Vec::new();
        check_required_preamble(
            "CHANGELOG.md",
            "\n\n# Changelog\n\n## Unreleased\n\n- an entry\n",
            &policy,
            &mut findings,
        );
        assert!(findings.is_empty(), "unexpected findings: {findings:?}");
    }
}
