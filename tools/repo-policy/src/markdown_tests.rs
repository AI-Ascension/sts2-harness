// SPDX-License-Identifier: MIT

//! Unit and control tests for the `DOC003` required-preamble rule and `DOC002` link targets.
//!
//! Split out of `markdown.rs` by the same `#[path]` convention the other
//! oversized modules in this crate use, so neither file exceeds the
//! repository's preferred source-size budget.

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

/// The exact blind spot that made the rule inert for its own subject:
/// `CHANGELOG.md` *describes* its `## Unreleased` heading in prose, so a
/// whole-line match is still present after the heading is deleted. A
/// substring match reported nothing here, and the size gate cannot see it
/// either when the deleted line is replaced so the count is unchanged.
#[test]
fn a_prose_mention_does_not_satisfy_a_missing_heading() {
    let policy = policy_with_changelog_markers();
    let mut findings = Vec::new();
    check_required_preamble(
        "CHANGELOG.md",
        "# Changelog\n\n- an entry describing the `## Unreleased` heading\n",
        &policy,
        &mut findings,
    );
    assert_eq!(
        findings.len(),
        1,
        "only the heading may be reported; the title is present: {findings:?}"
    );
    assert_eq!(findings[0].rule, "DOC003");
    assert!(
        findings[0].message.contains("## Unreleased"),
        "the finding must name the missing marker: {findings:?}"
    );
}

/// The doc comment has always claimed the markers are checked "in order".
/// `contains` never enforced that, so a reversed preamble passed.
#[test]
fn markers_out_of_their_declared_order_are_reported() {
    let policy = policy_with_changelog_markers();
    let mut findings = Vec::new();
    check_required_preamble(
        "CHANGELOG.md",
        "## Unreleased\n\n# Changelog\n\n- an entry\n",
        &policy,
        &mut findings,
    );
    // Both are reported, and for the two distinct reasons: the title no
    // longer opens the file, and the `## Unreleased` heading that belongs
    // after it was already consumed out of order.
    assert_eq!(
        findings.len(),
        2,
        "a reversed preamble breaks both the order and the opening anchor: {findings:?}"
    );
    assert!(
        findings.iter().all(|finding| finding.rule == "DOC003"),
        "unexpected rule: {findings:?}"
    );
    assert!(
        findings[0].message.contains("not the file's opening line"),
        "the first marker must be reported as displaced: {findings:?}"
    );
    assert!(
        findings[1].message.contains("## Unreleased"),
        "the second marker must be reported as missing: {findings:?}"
    );
}

/// One repeated heading must not satisfy the whole preamble.
#[test]
fn a_repeated_marker_does_not_stand_in_for_a_missing_one() {
    let policy = policy_with_changelog_markers();
    let mut findings = Vec::new();
    check_required_preamble(
        "CHANGELOG.md",
        "# Changelog\n\n# Changelog\n\n- an entry\n",
        &policy,
        &mut findings,
    );
    assert_eq!(
        findings.len(),
        1,
        "the second `# Changelog` is not `## Unreleased`: {findings:?}"
    );
    assert_eq!(findings[0].rule, "DOC003");
}

/// A title that survives but has been pushed below the content has still
/// lost the identity this rule asserts, so the first marker is anchored to
/// the file's first nonblank line rather than merely required to appear.
#[test]
fn a_title_that_no_longer_opens_the_file_is_reported() {
    let policy = policy_with_changelog_markers();
    let mut findings = Vec::new();
    check_required_preamble(
        "CHANGELOG.md",
        "- an entry\n\n# Changelog\n\n## Unreleased\n",
        &policy,
        &mut findings,
    );
    assert_eq!(
        findings.len(),
        1,
        "both markers appear, but the file no longer opens with its title: {findings:?}"
    );
    assert_eq!(findings[0].rule, "DOC003");
}

/// A file that opens with a blank line is still well formed: the anchor is
/// the first *nonblank* line, so ordinary leading whitespace is not a
/// finding.
#[test]
fn leading_blank_lines_do_not_break_the_opening_anchor() {
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
